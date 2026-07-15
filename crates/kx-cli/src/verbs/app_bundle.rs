//! Portable App bundles — `kx app export --bundle`, `kx app import`, `kx app clone`.
//!
//! A bundle is a `kortecx.appbundle/v1` archive: the canonical App envelope plus the
//! base64 closure of every content-store blob it references. All three flows are
//! LOCAL and client-orchestrated over the existing RPCs (`GetApp` / `GetContent` /
//! `PutContent` / `SaveApp`) — no dedicated server import RPC:
//!
//! - **export** GetApps the App, walks `env.content_refs(..)`, fetches each blob at
//!   FULL size via `GetContent` (the batch RPC clamps items at 512 KiB), and writes
//!   a bundle named by the App's `app_digest`.
//! - **import** re-validates + re-canonicalizes the envelope, verifies the declared
//!   `app_digest`, shows a review of the carried instruction bodies + requested
//!   capabilities, `PutContent`s each blob (the server re-derives + dedups the ref),
//!   then `SaveApp`s under the importer's OWN principal with a `source_digest`
//!   lineage stamp. Connections/secrets never travel — the importer re-registers by
//!   name (reported; fail-closed at run until then).
//! - **clone** is import's local cousin: `GetApp` → rename → `SaveApp` (content is
//!   already resident), recording the source's `app_digest` lineage.
//!
//! Security is inherited from the server RPCs (`SaveApp` re-validates the envelope —
//! the secret-leak refusal; `PutContent` re-derives every ref + caps each blob at
//! 32 MiB) plus these client-side guards: a body-revealing review (the confused-
//! deputy surface is imported prompt/rule/skill TEXT, not names), an export-side
//! best-effort secret scan, bundle-bomb ceilings, and no-silent-clobber. The
//! `source_digest` is a lineage HINT, never authenticity (OSS has no signing).

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::Path;

use kx_app::AppEnvelope;
use kx_appbundle::AppBundle;
use kx_content::ContentRef;
use kx_gateway_core::app_digest_of;
use kx_proto::proto;

use crate::client::Resolved;
use crate::error::CliError;
use crate::verbs::app;

type Client = proto::kx_gateway_client::KxGatewayClient<tonic::transport::Channel>;

/// Advisory client-side import ceilings (H7). Distinct from the server's per-blob
/// 32 MiB `PutContent` cap; these bound the WHOLE closure a bundle can carry.
const MAX_BUNDLE_REFS: usize = 4096;
/// 512 MiB — a portable App is documents + prompts, not a data lake.
const MAX_BUNDLE_CLOSURE_BYTES: u64 = 512 * 1024 * 1024;

/// Hex-encode a server-returned 32-byte content ref (fail-closed on a bad length).
fn ref_hex(bytes: &[u8]) -> Result<String, CliError> {
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| CliError::Usage("server returned a malformed content ref".into()))?;
    Ok(ContentRef(arr).to_hex())
}

/// `export --bundle`: write a portable content-closure archive for `handle`.
///
/// # Errors
/// [`CliError`] on a transport/status failure, a not-found App, a missing content
/// blob (cannot export faithfully), or a secret-looking blob without `--force`.
pub(super) async fn export_bundle(
    client: &mut Client,
    resolved: &Resolved,
    handle: &str,
    with_data: bool,
    force: bool,
    out: &Path,
) -> Result<(), CliError> {
    let resp = app::fetch_app(client, resolved, handle).await?;
    if !resp.found {
        return Err(CliError::Usage(format!("app {handle:?} not found")));
    }
    let env = AppEnvelope::from_json_slice(&resp.envelope_json)
        .map_err(|e| CliError::Usage(format!("stored envelope is invalid: {e}")))?;
    let canonical = env
        .to_canonical_json()
        .map_err(|e| CliError::Usage(format!("serialize envelope: {e}")))?;
    let app_digest = ContentRef(app_digest_of(&canonical)).to_hex();

    // Fetch each closure blob at FULL size (GetContent — the batch RPC clamps at 512 KiB).
    let mut blobs = BTreeMap::new();
    for hexref in env.content_refs(with_data) {
        let cref = ContentRef::from_hex(&hexref).ok_or_else(|| {
            CliError::Usage(format!(
                "stored envelope has a malformed content ref {hexref:?}"
            ))
        })?;
        let blob = client
            .get_content(resolved.request(proto::GetContentRequest {
                content_ref: cref.0.to_vec(),
                instance_id: Vec::new(), // uploads scope (authoring artifacts)
            })?)
            .await
            .map_err(CliError::from_status)?
            .into_inner();
        if blob.payload.is_empty() {
            return Err(CliError::Usage(format!(
                "content {hexref} is missing or unreadable — cannot export a faithful bundle"
            )));
        }
        blobs.insert(hexref, blob.payload);
    }

    let bundle = AppBundle {
        app_digest,
        source_digest: None,
        envelope: canonical,
        blobs,
    };
    scan_blobs_for_secrets(&bundle, force)?;
    let wire = bundle
        .to_json()
        .map_err(|e| CliError::Usage(format!("serialize bundle: {e}")))?;
    std::fs::write(out, wire.as_bytes())
        .map_err(|e| CliError::Usage(format!("write {}: {e}", out.display())))?;
    println!(
        "wrote {} ({} blob(s), {} bytes)",
        out.display(),
        bundle.blob_count(),
        bundle.total_blob_bytes()
    );
    Ok(())
}

/// `import <bundle>`: reconcile a bundle fail-closed under the caller's own principal.
///
/// # Errors
/// [`CliError`] on a malformed/oversized/tampered bundle, a declined review, an
/// existing App without `--force`, a blob integrity mismatch, or a transport failure.
pub(super) async fn import_bundle(
    client: &mut Client,
    resolved: &Resolved,
    path: &Path,
    yes: bool,
    force: bool,
) -> Result<(), CliError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| CliError::Usage(format!("cannot read {}: {e}", path.display())))?;
    let bundle = AppBundle::from_json(&raw)
        .map_err(|e| CliError::Usage(format!("invalid app bundle: {e}")))?;

    // Bundle-bomb ceilings (H7) — BEFORE any upload.
    if bundle.blob_count() > MAX_BUNDLE_REFS {
        return Err(CliError::Usage(format!(
            "bundle carries {} blobs (ceiling {MAX_BUNDLE_REFS}) — refusing",
            bundle.blob_count()
        )));
    }
    if bundle.total_blob_bytes() > MAX_BUNDLE_CLOSURE_BYTES {
        return Err(CliError::Usage(format!(
            "bundle closure is {} bytes (ceiling {MAX_BUNDLE_CLOSURE_BYTES}) — refusing",
            bundle.total_blob_bytes()
        )));
    }

    // Re-validate + re-canonicalize; verify the declared digest matches the envelope
    // (a corruption/tamper check — the server re-validates the envelope at SaveApp too).
    let env = AppEnvelope::from_json_slice(&bundle.envelope)
        .map_err(|e| CliError::Usage(format!("bundle envelope is invalid: {e}")))?;
    let canonical = env
        .to_canonical_json()
        .map_err(|e| CliError::Usage(format!("serialize envelope: {e}")))?;
    let derived = ContentRef(app_digest_of(&canonical)).to_hex();
    if derived != bundle.app_digest {
        return Err(CliError::Usage(format!(
            "corrupt bundle: declared app_digest {} != the envelope's actual digest {derived}",
            bundle.app_digest
        )));
    }

    // Body-revealing review + confirmation (the confused-deputy surface, H1).
    review_and_confirm(&env, &bundle, yes)?;

    // Handle from the envelope name (the server also validates it); no silent clobber (H4).
    let handle = app::default_handle(&env.name);
    if !force && app::fetch_app(client, resolved, &handle).await?.found {
        return Err(CliError::Usage(format!(
            "app {handle:?} already exists — pass --force to overwrite"
        )));
    }

    // PutContent each blob (server re-derives the ref + dedups); verify ref == declared key.
    let mut deduped = 0usize;
    for (hexref, bytes) in &bundle.blobs {
        let put = client
            .put_content(resolved.request(proto::PutContentRequest {
                payload: bytes.clone(),
                media_type: String::new(),
                filename: String::new(),
            })?)
            .await
            .map_err(CliError::from_status)?
            .into_inner();
        if &ref_hex(&put.content_ref)? != hexref {
            return Err(CliError::Usage(format!(
                "corrupt bundle: a blob was declared as {hexref} but the store derived a different ref"
            )));
        }
        if put.deduplicated {
            deduped += 1;
        }
    }

    // SaveApp under the importer's OWN principal, stamping local lineage (the source digest).
    let source_digest = ContentRef::from_hex(&bundle.app_digest)
        .map(|c| c.0.to_vec())
        .unwrap_or_default();
    client
        .save_app(resolved.request(proto::SaveAppRequest {
            handle: handle.clone(),
            envelope_json: canonical,
            source_digest,
        })?)
        .await
        .map_err(CliError::from_status)?;

    println!(
        "imported {handle} ({} blob(s), {deduped} already present)",
        bundle.blob_count()
    );
    report_unsatisfiable(&env, &bundle);
    Ok(())
}

/// `clone <handle> <newname>`: a local frozen copy under a new name.
///
/// # Errors
/// [`CliError`] on a not-found source, an invalid rename, an existing target, or a
/// transport failure.
pub(super) async fn clone_app(
    client: &mut Client,
    resolved: &Resolved,
    handle: &str,
    newname: &str,
) -> Result<(), CliError> {
    let resp = app::fetch_app(client, resolved, handle).await?;
    if !resp.found {
        return Err(CliError::Usage(format!("app {handle:?} not found")));
    }
    // The source's app_digest is the clone's lineage (recompute if the server omitted it).
    let source_digest = if resp.app_digest.len() == 32 {
        resp.app_digest.clone()
    } else {
        app_digest_of(&resp.envelope_json).to_vec()
    };
    let mut env = AppEnvelope::from_json_slice(&resp.envelope_json)
        .map_err(|e| CliError::Usage(format!("stored envelope is invalid: {e}")))?;
    env.name = newname.to_string(); // rename ⇒ a new app_digest (with lineage back to the source)
    env.validate()
        .map_err(|e| CliError::Usage(format!("clone envelope is invalid: {e}")))?;
    let canonical = env
        .to_canonical_json()
        .map_err(|e| CliError::Usage(format!("serialize envelope: {e}")))?;
    let new_handle = app::default_handle(newname);
    if app::fetch_app(client, resolved, &new_handle).await?.found {
        return Err(CliError::Usage(format!(
            "app {new_handle:?} already exists — choose a different <newname>"
        )));
    }
    client
        .save_app(resolved.request(proto::SaveAppRequest {
            handle: new_handle.clone(),
            envelope_json: canonical,
            source_digest,
        })?)
        .await
        .map_err(CliError::from_status)?;
    println!("cloned {handle} → {new_handle}");
    Ok(())
}

/// Render the carried instruction BODIES + requested capabilities, then confirm.
/// The imported prompt/rule/skill text steers the model under the importer's OWN
/// granted tools — a name-only summary would make `--yes` security theater.
fn review_and_confirm(env: &AppEnvelope, bundle: &AppBundle, yes: bool) -> Result<(), CliError> {
    println!("Importing App {:?} (v{}).", env.name, env.version);
    if !env.description.is_empty() {
        println!("  {}", env.description);
    }
    let mut shown = false;
    for a in env.references.prompts.iter().chain(&env.references.rules) {
        if let Some(body) = bundle.blobs.get(&a.content_ref) {
            println!("\n--- carried instruction: {} ---", a.name);
            println!("{}", String::from_utf8_lossy(body));
            shown = true;
        }
    }
    for s in &env.references.skills {
        if let Some(body) = bundle.blobs.get(&s.instructions_ref) {
            println!("\n--- carried skill: {} ---", s.name);
            println!("{}", String::from_utf8_lossy(body));
            shown = true;
        }
    }
    if !shown {
        println!("  (no carried prompt/rule/skill instruction bodies)");
    }
    let conns: Vec<&str> = env
        .references
        .connections
        .iter()
        .map(|c| c.credential_ref.as_str())
        .filter(|s| !s.is_empty())
        .collect();
    if !conns.is_empty() {
        println!(
            "\nRequests connections (re-register locally by name): {}",
            conns.join(", ")
        );
    }
    let tools: Vec<&str> = env
        .steering_config
        .tools
        .requested_grants
        .keys()
        .map(String::as_str)
        .collect();
    if !tools.is_empty() {
        println!(
            "Requests tools (granted only if you already hold them): {}",
            tools.join(", ")
        );
    }
    println!(
        "\nImported instructions run under YOUR OWN granted tools. Review them before proceeding."
    );

    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(CliError::Usage(
            "refusing to import non-interactively without confirmation — re-run with --yes \
             after reviewing the instructions above"
                .into(),
        ));
    }
    print!("Proceed with import? [y/N] ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| CliError::Usage(format!("read confirmation: {e}")))?;
    if !matches!(line.trim(), "y" | "Y" | "yes" | "YES") {
        return Err(CliError::Usage("import aborted".into()));
    }
    Ok(())
}

/// Report references that don't travel (connections/tools/datasets) — the importer
/// re-registers them locally; the App fails closed at run until then (SOFT preflight).
///
/// A dataset whose corpus TRAVELLED in the bundle is omitted: it is self-contained and
/// materializes itself on first run (`T-RUNAPP-RAG-SELF-CONTAINED`), so listing it as a
/// re-ingest chore would be a lie. A dataset naming `cas_refs` whose blobs did NOT travel
/// (an export without `--with-data`) still needs a local re-ingest, and still says so.
fn report_unsatisfiable(env: &AppEnvelope, bundle: &AppBundle) {
    let todos = unsatisfiable_todos(env, bundle);
    if !todos.is_empty() {
        println!(
            "\nThis App references integrations you may need to register locally \
             (it fails closed at run until then):"
        );
        for t in &todos {
            println!("  - {t}");
        }
    }
}

/// The pure half of [`report_unsatisfiable`] — the chores an import leaves behind, in
/// declaration order. Pure ⇒ unit-testable without capturing stdout (Rule 5.2).
fn unsatisfiable_todos(env: &AppEnvelope, bundle: &AppBundle) -> Vec<String> {
    let mut todos = Vec::new();
    for c in &env.references.connections {
        if !c.credential_ref.is_empty() {
            todos.push(format!(
                "connection credential {:?} (register with `kx connections add`)",
                c.credential_ref
            ));
        }
    }
    for t in &env.references.tools {
        todos.push(format!("tool {}@{}", t.tool_id, t.tool_version));
    }
    for d in &env.references.datasets {
        let travelled =
            !d.cas_refs.is_empty() && d.cas_refs.iter().all(|r| bundle.blobs.contains_key(r));
        if !travelled {
            todos.push(format!("dataset {:?} (re-ingest locally)", d.dataset_ref));
            continue;
        }
        // It travelled — but "travelled" only implies "self-materializes" while it fits under
        // the server's self-ingest ceiling. Above it, the gateway fail-soft SKIPS the ingest
        // (`kx-gateway`'s `ensure_app_dataset`) and the App grounds on nothing — announced only
        // by a `tracing::warn!` on a server, long after the author could act.
        //
        // So the doc above INVERTS here: omitting an over-ceiling dataset because "listing it
        // would be a lie" is itself the lie. This is the one place the size is known while the
        // user can still do something about it (split the corpus, or pre-ingest it on the
        // target). `MAX_BUNDLE_CLOSURE_BYTES` cannot catch it — 512 MiB bounds the WHOLE
        // closure, so a 200 MiB corpus sails through and then silently does not ground.
        let bytes: u64 = d
            .cas_refs
            .iter()
            .filter_map(|r| bundle.blobs.get(r))
            .map(|b| b.len() as u64)
            .sum();
        if bytes > kx_app::MAX_APP_CORPUS_BYTES {
            todos.push(format!(
                "dataset {:?} carries {bytes} bytes — OVER the {} byte self-ingest ceiling, so \
                 it will NOT materialize on first run (the App will ground on an existing \
                 dataset of that name, or on nothing). Split the corpus or pre-ingest it.",
                d.dataset_ref,
                kx_app::MAX_APP_CORPUS_BYTES
            ));
        } else if d.cas_refs.len() > kx_app::MAX_APP_CORPUS_REFS {
            todos.push(format!(
                "dataset {:?} names {} refs — OVER the {} ref self-ingest ceiling, so it will \
                 NOT materialize on first run. Split the corpus or pre-ingest it.",
                d.dataset_ref,
                d.cas_refs.len(),
                kx_app::MAX_APP_CORPUS_REFS
            ));
        }
    }
    todos
}

/// Refuse to export blobs whose bodies look like they carry a secret (bundle blob
/// bodies travel VERBATIM) unless `--force`. Best-effort + high-signal — NOT a
/// guarantee (the envelope leak-scan covers structure, not blob CONTENTS).
fn scan_blobs_for_secrets(bundle: &AppBundle, force: bool) -> Result<(), CliError> {
    let hits: Vec<&String> = bundle
        .blobs
        .iter()
        .filter(|(_, body)| looks_like_secret(body))
        .map(|(r, _)| r)
        .collect();
    if hits.is_empty() {
        return Ok(());
    }
    eprintln!(
        "warning: {} blob(s) look like they contain a secret/key — bundle bodies travel VERBATIM:",
        hits.len()
    );
    for r in &hits {
        eprintln!("  - {}…", &r[..12]);
    }
    if force {
        Ok(())
    } else {
        Err(CliError::Usage(
            "refusing to export blobs that look like secrets; review, then re-run with --force"
                .into(),
        ))
    }
}

/// High-signal, low-false-positive secret detection over a blob body.
fn looks_like_secret(body: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(body) else {
        return false;
    };
    if text.contains("-----BEGIN ") {
        return true; // PEM private-key blocks
    }
    // AWS access key id: `AKIA` + 16 uppercase-alnum.
    if let Some(i) = text.find("AKIA") {
        let tail: String = text[i + 4..].chars().take(16).collect();
        if tail.len() == 16
            && tail
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return true;
        }
    }
    // Provider-style token: `sk-` + a run of ≥20 alphanumerics.
    if let Some(i) = text.find("sk-") {
        let run = text[i + 3..]
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .count();
        if run >= 20 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_scan_flags_high_signal_patterns() {
        assert!(looks_like_secret(
            b"-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END-----"
        ));
        assert!(looks_like_secret(b"aws_key = AKIAIOSFODNN7EXAMPLE"));
        assert!(looks_like_secret(
            b"token: sk-abcdefghijklmnopqrstuvwxyz0123"
        ));
        // Benign prose must NOT trip the scan (low false positives).
        assert!(!looks_like_secret(
            b"Never reveal the user's password or any secret token."
        ));
        assert!(!looks_like_secret(b"Summarize the document."));
        assert!(!looks_like_secret(&[0u8, 1, 2, 255])); // non-UTF-8
    }

    #[test]
    fn ref_hex_round_trips_and_rejects_bad_length() {
        let r = ContentRef([0xabu8; 32]);
        assert_eq!(ref_hex(&r.0).unwrap(), r.to_hex());
        assert!(ref_hex(&[0u8; 16]).is_err());
    }

    /// A dataset whose corpus TRAVELLED is self-contained — it self-ingests on first run,
    /// so telling the importer to "re-ingest locally" would be a lie. Everything that did
    /// NOT travel still says so.
    #[test]
    fn a_travelled_corpus_is_not_reported_as_a_re_ingest_chore() {
        let carried = "aa".repeat(32);
        let stranded = "bb".repeat(32);
        let mut env = AppEnvelope::new("a", serde_json::json!({ "steps": [] }));
        for (name, refs) in [
            ("carried", vec![carried.clone()]),   // travelled ⇒ omitted
            ("stranded", vec![stranded.clone()]), // named, never shipped ⇒ listed
            ("bare", vec![]),                     // reference-existing ⇒ listed
        ] {
            env.references.datasets.push(kx_app::DatasetRef {
                dataset_ref: name.into(),
                cas_refs: refs,
            });
        }
        let mut bundle = AppBundle {
            app_digest: String::new(),
            source_digest: None,
            envelope: Vec::new(),
            blobs: BTreeMap::default(),
        };
        bundle.blobs.insert(carried, b"body".to_vec());

        let todos = unsatisfiable_todos(&env, &bundle);
        assert_eq!(
            todos,
            vec![
                "dataset \"stranded\" (re-ingest locally)".to_string(),
                "dataset \"bare\" (re-ingest locally)".to_string(),
            ]
        );
    }

    /// …but "it travelled" only means "it self-materializes" while it FITS. Above the server's
    /// self-ingest ceiling the gateway fail-soft SKIPS the ingest and the App grounds on
    /// nothing — announced only by a `tracing::warn!` on a server, long after the author could
    /// act. So for an over-ceiling corpus, staying silent (the case above) is the lie.
    ///
    /// `MAX_BUNDLE_CLOSURE_BYTES` (512 MiB, the WHOLE closure) cannot catch this: a corpus
    /// between 64 MiB and 512 MiB exports perfectly cleanly and then never grounds. Export is
    /// the only moment the size is known while the user can still split or pre-ingest it.
    #[test]
    fn a_travelled_corpus_over_the_self_ingest_ceiling_is_reported_at_export() {
        let over = "cc".repeat(32);
        let under = "dd".repeat(32);
        let mut env = AppEnvelope::new("a", serde_json::json!({ "steps": [] }));
        for (name, refs) in [
            ("huge", vec![over.clone()]), // travelled but OVER the byte ceiling ⇒ reported
            ("small", vec![under.clone()]), // travelled and fits ⇒ silent, as before
        ] {
            env.references.datasets.push(kx_app::DatasetRef {
                dataset_ref: name.into(),
                cas_refs: refs,
            });
        }
        let mut bundle = AppBundle {
            app_digest: String::new(),
            source_digest: None,
            envelope: Vec::new(),
            blobs: BTreeMap::default(),
        };
        // One byte over is enough — the boundary is `>`, so exactly-at-ceiling stays silent.
        // `try_from`, not `as`: an `as` cast silently truncates on a 32-bit target, which would
        // turn this fixture into an UNDER-ceiling one and quietly invert the test into a green
        // that proves the opposite. Panic loudly there instead (clippy's cast_possible_truncation
        // caught this under CI's -D warnings).
        let over_len = usize::try_from(kx_app::MAX_APP_CORPUS_BYTES)
            .expect("the corpus ceiling must fit in usize on this target")
            + 1;
        bundle.blobs.insert(over, vec![b'x'; over_len]);
        bundle.blobs.insert(under, b"body".to_vec());

        let todos = unsatisfiable_todos(&env, &bundle);
        assert_eq!(
            todos.len(),
            1,
            "only the over-ceiling dataset is reported: {todos:?}"
        );
        assert!(
            todos[0].starts_with("dataset \"huge\" carries"),
            "{todos:?}"
        );
        assert!(
            todos[0].contains("will NOT materialize on first run"),
            "the warning must say what will HAPPEN, not just cite a number: {todos:?}"
        );
    }
}
