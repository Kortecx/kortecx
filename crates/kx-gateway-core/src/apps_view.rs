//! The POC-4 App-catalog seam behind `SaveApp` / `ListApps` / `GetApp`.
//!
//! An "App" is a `kortecx.app/v1` envelope (a portable blueprint wrapped with
//! by-REFERENCE references, a 4-axis steering config, and replay intent). Spoken
//! in gateway-core's own wire vocabulary — **opaque envelope BYTES** + a
//! host-derived [`AppRecord`] summary + a `[u8; 16]` ref. No envelope type crosses
//! the seam, so gateway-core never links `kx-app`; the host (`kx-gateway`)
//! canonicalizes + validates the envelope and derives the summary + `app_ref`.
//!
//! # Boundaries (load-bearing)
//!
//! - **Off the truth path.** The `apps.db` sidecar is REBUILDABLE-TO-EMPTY (the
//!   `bundles.db`/D160 posture): an App envelope references content-store blobs +
//!   registry ids; it is NOT journal-derivable. Never journaled, never a `MoteId`
//!   input, never a digest input — dropping the file cannot move the canonical
//!   projection digest.
//! - **Carries NO authority (SN-8 / BLOCKER #5).** The envelope holds references +
//!   an authorship claim only — `app run` re-compiles the blueprint and the server
//!   re-resolves every warrant from the caller's OWN grants. The host validates
//!   that the envelope carries no warrant/grant/secret/credential/`instance_id`.
//! - **Server-derived id.** `app_ref = blake3("kx-app\0" ‖ handle ‖ canonical(envelope))[..16]`;
//!   the client names a handle, never an identity. The host re-canonicalizes the
//!   received bytes so client byte-ordering never affects identity.
//! - **Caller-scoped.** Every method takes the SERVER-RESOLVED `principal`; an App
//!   is visible only to the party that authored it (uniform not-found for absent OR
//!   not-owned — no cross-party existence oracle).
//! - **`None` seam ⇒ degrade.** A host without the sidecar leaves the three RPCs
//!   `unimplemented` (a clear, fail-closed signal).
//! - **No cross-instance import** in this seam (a sharing feature, deferred).

use kx_content::ContentRef;

use crate::error::GatewayError;

/// Fail-closed cap on a single App envelope's serialized size (checked at the
/// `SaveApp` handler BEFORE any host touch).
pub const MAX_APP_ENVELOPE_BYTES: usize = 1 << 20; // 1 MiB

/// Domain-separation tag for the handle-free App identity ([`app_digest_of`]). The exact
/// preimage — `blake3(APP_DIGEST_DOMAIN ‖ canonical_envelope)` — is a stable, versioned
/// contract: every producer of an `app_digest` (the runtime, an SDK) MUST compute it
/// byte-for-byte identically so the digest names the SAME App everywhere. Changing the
/// algorithm bumps the `/vN` tag (a new digest namespace), never a silent redefinition.
pub const APP_DIGEST_DOMAIN: &[u8] = b"kortecx.app-digest/v1\0";

/// `app_digest = blake3(APP_DIGEST_DOMAIN ‖ canonical_envelope)` — the FULL 32-byte,
/// HANDLE-FREE identity of an App.
///
/// Unlike `app_ref` (the host folds in the save handle + truncates to 16B for local catalog
/// dedup), `app_digest` is IDENTICAL for byte-identical envelopes no matter which handle or
/// principal they are stored under — a stable, portable identity for the App itself.
/// Exact-equality only (SN-8); never a similarity key.
///
/// Stability: a pure function of the canonical envelope bytes — any field intentionally
/// excluded from identity must be stripped before hashing; today the envelope carries no
/// such field, so the input is the canonical envelope verbatim.
#[must_use]
pub fn app_digest_of(canonical: &[u8]) -> [u8; 32] {
    let mut keyed = Vec::with_capacity(APP_DIGEST_DOMAIN.len() + canonical.len());
    keyed.extend_from_slice(APP_DIGEST_DOMAIN);
    keyed.extend_from_slice(canonical);
    ContentRef::of(&keyed).0
}

/// Domain-separation tag for an App's SELF-CONTAINED dataset scope
/// ([`app_dataset_scoped_name`]). Same versioned-contract discipline as
/// [`APP_DIGEST_DOMAIN`]: a preimage change bumps the `/vN` tag (a NEW name
/// namespace) rather than silently re-pointing every App at a different index.
pub const APP_DATASET_SCOPE_DOMAIN: &[u8] = b"kortecx.app-dataset-scope/v1\0";

/// The longest readable prefix kept from the declared `dataset_ref`. Bounded so the
/// composed name always fits the host's 128-char dataset-name cap: `100 + ".app-" + 8`
/// = 113. The hash carries the identity, so truncating the DISPLAY half is collision-safe.
const MAX_READABLE_CHARS: usize = 100;

/// Append a length-prefixed field to a hash preimage, so two fields can never collide
/// by concatenation (`("x", ["y"])` vs `("xy", [])`). Mirrors `index_fingerprint`'s
/// discipline in `kx-dataset`.
fn push_field(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    buf.extend_from_slice(bytes);
}

/// The readable half of a scoped name: the declared `dataset_ref` reduced to the host's
/// allowed alphabet. Non-`[A-Za-z0-9._-]` maps to `-` (real refs carry `/`), and the map
/// runs BEFORE the truncate so every kept char is ASCII — slicing a raw multi-byte
/// `dataset_ref` at a byte bound would panic.
///
/// Leading dots are dropped and an empty result falls back to `ds`, so a scoped name
/// never leads with `.` — the host rejects a BARE dot run, and a `.`-leading name reads
/// as a hidden file to a human scanning `kx datasets list`. Display-only: the hash is
/// taken over the RAW ref, so `.hidden` and `hidden` still name different indices.
fn readable_prefix(dataset_ref: &str) -> String {
    let cleaned: String = dataset_ref
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(MAX_READABLE_CHARS)
        .collect();
    let trimmed = cleaned.trim_start_matches('.');
    if trimmed.is_empty() {
        "ds".to_string()
    } else {
        trimmed.to_string()
    }
}

/// The physical dataset name an App's SELF-CONTAINED corpus is ingested under —
/// `<readable>.app-<hash8>` (e.g. `science.app-3f2a9c81`). Pure + total: the output
/// always satisfies the host's `validate_dataset_name` for ANY input (proven by property
/// test), so a caller never has to pre-sanitize.
///
/// # Why scope at all
/// Ingesting an imported App's corpus under its BARE declared name would silently MERGE
/// it into a same-named local dataset (the host keys datasets by a bare name, and ingest
/// is insert-or-ignore) — permanently, with no delete RPC to undo it. The scope is
/// collision-AVOIDANCE, not a security boundary: a hosted OSS server is single-tenant by
/// construction (the cross-party wall is the cloud layer above).
///
/// # The preimage (a versioned contract)
/// `blake3(APP_DATASET_SCOPE_DOMAIN ‖ ⟨scope_tag⟩ ‖ ⟨dataset_ref⟩ ‖ n ‖ ⟨cas_ref⟩…)`,
/// where `⟨x⟩` is length-prefixed and `cas_refs` are SORTED + DEDUPED — so the name is a
/// function of the corpus SET, invariant to declaration order and duplicates. First 8 hex
/// (32 bits) of the digest; ample for a single-tenant local store, and short enough that a
/// model can copy the name back into `retrieve@1` verbatim.
///
/// Keyed on `(scope_tag, dataset_ref, corpus)`, deliberately NOT on `app_digest`:
/// - `app_digest` changes on ANY envelope edit ⇒ the name would rotate ⇒ grounding lost
///   on every re-save. Corpus-derived survives envelope edits.
/// - `scope_tag` is the host's live embed scope (model/pooling/chunk/tokenizer). Folding it
///   in is the STALE-INDEX ESCAPE: ingest refuses to mix embed spaces and no RPC can drop a
///   dataset, so a name that never rotated would be permanently unrecoverable after a model
///   swap. A swap now yields a NEW name ⇒ auto re-ingest ⇒ self-healing; the old index
///   simply orphans (inert — `datasets.db` is a rebuildable-to-EMPTY sidecar).
#[must_use]
pub fn app_dataset_scoped_name(scope_tag: &str, dataset_ref: &str, cas_refs: &[String]) -> String {
    let mut refs: Vec<&str> = cas_refs.iter().map(String::as_str).collect();
    refs.sort_unstable();
    refs.dedup();

    let mut preimage = APP_DATASET_SCOPE_DOMAIN.to_vec();
    push_field(&mut preimage, scope_tag.as_bytes());
    push_field(&mut preimage, dataset_ref.as_bytes());
    preimage.extend_from_slice(&(refs.len() as u64).to_le_bytes());
    for r in refs {
        push_field(&mut preimage, r.as_bytes());
    }
    let hash = ContentRef::of(&preimage).to_hex();
    format!("{}.app-{}", readable_prefix(dataset_ref), &hash[..8])
}

/// A stored App's summary — the catalog/display view. The envelope bytes are
/// opaque to gateway-core; the host derives every field from the canonical JSON.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppRecord {
    /// 16-byte SERVER-DERIVED canonical-envelope hash (display + dedup signal).
    pub app_ref: [u8; 16],
    /// The canonical `namespace/collection/name` handle (the upsert key).
    pub handle: String,
    /// Envelope name.
    pub name: String,
    /// Envelope version.
    pub version: String,
    /// Advisory description (never parsed for enforcement).
    pub description: String,
    /// Catalog tags.
    pub tags: Vec<String>,
    /// Blueprint step count (display only).
    pub step_count: u32,
    /// OPTIONAL 32-byte lineage hint — the `app_digest` this App was imported/cloned
    /// from (`None` ⇒ authored-here). Off-identity (never in the `app_ref`/`app_digest`
    /// preimage), off-journal, off-digest. A provenance hint, never authenticity.
    pub source_digest: Option<Vec<u8>>,
}

/// The App-catalog store seam: save / enumerate / fetch a caller's App envelopes.
/// Opaque envelope bytes cross the seam; identity + summary are host-derived. A
/// `None` seam on the service ⇒ the three RPCs return `unimplemented`.
pub trait AppCatalog: Send + Sync {
    /// Upsert the envelope bound to `(principal, handle)`. The host validates +
    /// canonicalizes `envelope_json`, derives `app_ref` + the summary, and stores
    /// the canonical bytes. `source_digest` is an OPTIONAL 32-byte off-identity
    /// lineage hint (an import/clone records the source's `app_digest`; `None` ⇒
    /// authored-here) — it never affects `app_ref` or dedup. Returns
    /// `(record, deduplicated)` where `deduplicated` is `true` iff an identical
    /// canonical envelope was already bound here.
    ///
    /// # Errors
    /// [`GatewayError::InvalidArgument`] if the envelope fails validation;
    /// [`GatewayError::Internal`] on a host write failure.
    fn save(
        &self,
        principal: &str,
        handle: &str,
        envelope_json: &[u8],
        source_digest: Option<&[u8]>,
    ) -> Result<(AppRecord, bool), GatewayError>;

    /// List `principal`'s apps in deterministic handle order, paged. Returns
    /// `(records, has_more)`; `after_handle` is an exclusive cursor.
    ///
    /// # Errors
    /// A host read failure ([`GatewayError::Internal`]).
    fn list(
        &self,
        principal: &str,
        limit: usize,
        after_handle: Option<&str>,
    ) -> Result<(Vec<AppRecord>, bool), GatewayError>;

    /// Fetch `(record, canonical_envelope_bytes)` bound to `(principal, handle)`,
    /// if any (caller-scoped; uniform not-found for absent OR not-owned).
    ///
    /// # Errors
    /// A host read failure ([`GatewayError::Internal`]).
    fn get(
        &self,
        principal: &str,
        handle: &str,
    ) -> Result<Option<(AppRecord, Vec<u8>)>, GatewayError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// `app_digest_of` is a PURE, deterministic function of its input bytes, and
        /// equals the exact `blake3(APP_DIGEST_DOMAIN ‖ bytes)` contract for ANY input
        /// (SN-4 v2 #5 — property test over the arbitrary byte space, not hand-picked cases).
        #[test]
        fn app_digest_of_is_pure_and_matches_the_contract(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
            prop_assert_eq!(app_digest_of(&bytes), app_digest_of(&bytes));
            let mut preimage = APP_DIGEST_DOMAIN.to_vec();
            preimage.extend_from_slice(&bytes);
            prop_assert_eq!(app_digest_of(&bytes), ContentRef::of(&preimage).0);
        }
    }

    /// The host's dataset-name contract, restated locally: gateway-core cannot call
    /// `validate_dataset_name` (it lives in the host crate, and the dep only runs the
    /// other way). The REAL validator is pinned against this fn by a property test in
    /// `kx-gateway::datasets` — this is the shape assertion, that one is the contract.
    fn is_host_valid(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 128
            && name != "."
            && name != ".."
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    }

    fn hexref(b: u8) -> String {
        format!("{b:02x}").repeat(32)
    }

    proptest! {
        /// TOTALITY (SN-4 v2 #5): the scoped name is host-valid for ANY declared ref —
        /// unicode, separators, absurd length, empty. A caller never pre-sanitizes.
        #[test]
        fn scoped_name_is_always_host_valid(
            dataset_ref in ".*",
            tag in ".*",
            refs in prop::collection::vec("[0-9a-f]{64}", 0..4),
        ) {
            let name = app_dataset_scoped_name(&tag, &dataset_ref, &refs);
            prop_assert!(is_host_valid(&name), "not host-valid: {name:?}");
        }

        /// The name is a function of the corpus SET: invariant to declaration order and
        /// to duplicates (the preimage sorts + dedups).
        #[test]
        fn scoped_name_ignores_cas_ref_order_and_duplicates(
            mut refs in prop::collection::vec("[0-9a-f]{64}", 1..6),
        ) {
            let base = app_dataset_scoped_name("t", "science", &refs);
            refs.reverse();
            let dup = [refs.clone(), refs.clone()].concat();
            prop_assert_eq!(&base, &app_dataset_scoped_name("t", "science", &refs));
            prop_assert_eq!(&base, &app_dataset_scoped_name("t", "science", &dup));
        }
    }

    #[test]
    fn scoped_name_is_deterministic_and_readable_first() {
        let refs = vec![hexref(0x01)];
        let a = app_dataset_scoped_name("tag", "science", &refs);
        assert_eq!(a, app_dataset_scoped_name("tag", "science", &refs));
        // Readable half LEADS (the model copies a prefix into retrieve@1, not a hash).
        assert!(a.starts_with("science.app-"), "{a}");
        assert_eq!(a.len(), "science.app-".len() + 8);
    }

    #[test]
    fn scoped_name_keys_on_corpus_and_tag_and_declared_ref() {
        let one = vec![hexref(0x01)];
        let two = vec![hexref(0x02)];
        let base = app_dataset_scoped_name("tag", "science", &one);
        // A different corpus ⇒ a different index.
        assert_ne!(base, app_dataset_scoped_name("tag", "science", &two));
        // The STALE-INDEX escape: a live embed-scope swap rotates the name, so the App
        // re-ingests into a fresh index instead of resolving to an unqueryable one.
        assert_ne!(base, app_dataset_scoped_name("tag2", "science", &one));
        // A different declared name over the same corpus is a different binding.
        assert_ne!(base, app_dataset_scoped_name("tag", "physics", &one));
        // An empty corpus is still distinct from a populated one.
        assert_ne!(base, app_dataset_scoped_name("tag", "science", &[]));
    }

    #[test]
    fn scoped_name_length_prefixes_defeat_concatenation_collisions() {
        // ("x", ["y"]) vs ("xy", []) — distinct only because fields are length-prefixed.
        assert_ne!(
            app_dataset_scoped_name("x", "d", &[hexref(0x01)]),
            app_dataset_scoped_name("xd", "", &[hexref(0x01)])
        );
        assert_ne!(
            app_dataset_scoped_name("t", "ab", &[]),
            app_dataset_scoped_name("t", "a", &["b".to_string()])
        );
    }

    #[test]
    fn scoped_name_sanitizes_the_readable_half_without_losing_identity() {
        let refs = vec![hexref(0x01)];
        // Host-illegal chars (real refs carry `/`) map into the allowlist...
        let slashed = app_dataset_scoped_name("t", "team/ds/docs", &refs);
        assert!(slashed.starts_with("team-ds-docs.app-"), "{slashed}");
        assert!(is_host_valid(&slashed));
        // ...but the RAW ref is hashed, so a sanitize-collision is not a name collision.
        assert_ne!(slashed, app_dataset_scoped_name("t", "team.ds.docs", &refs));
        // An all-illegal / empty / dot-run ref still yields a valid, non-dot-leading name.
        for ref_name in ["", "日本語", ".", "..", ".hidden"] {
            let n = app_dataset_scoped_name("t", ref_name, &refs);
            assert!(is_host_valid(&n), "{ref_name:?} -> {n:?}");
            assert!(!n.starts_with('.'), "{ref_name:?} -> {n:?}");
        }
        // Trimming the display half is not a loss of identity.
        assert_ne!(
            app_dataset_scoped_name("t", ".hidden", &refs),
            app_dataset_scoped_name("t", "hidden", &refs)
        );
    }

    #[test]
    fn scoped_name_truncates_an_absurd_declared_ref_within_the_host_cap() {
        let refs = vec![hexref(0x01)];
        // Multi-byte: a truncate-BEFORE-sanitize impl would panic on a char boundary.
        let long_unicode = "日".repeat(10_000);
        let n = app_dataset_scoped_name("t", &long_unicode, &refs);
        assert!(is_host_valid(&n), "{n}");
        assert_eq!(n.len(), MAX_READABLE_CHARS + ".app-".len() + 8);
        // Two long refs sharing the truncated prefix stay DISTINCT (the hash is full-input).
        let a = format!("{}alpha", "x".repeat(MAX_READABLE_CHARS));
        let b = format!("{}beta", "x".repeat(MAX_READABLE_CHARS));
        assert_ne!(
            app_dataset_scoped_name("t", &a, &refs),
            app_dataset_scoped_name("t", &b, &refs)
        );
    }

    #[test]
    fn app_digest_is_deterministic_and_matches_the_domain_contract() {
        let canonical = br#"{"name":"x","schema":"kortecx.app/v1"}"#;
        // Deterministic + a pure function of the bytes.
        assert_eq!(app_digest_of(canonical), app_digest_of(canonical));
        assert_ne!(app_digest_of(canonical), app_digest_of(b"{}"));
        // The exact cross-runtime byte contract: blake3(APP_DIGEST_DOMAIN ‖ canonical).
        let mut preimage = APP_DIGEST_DOMAIN.to_vec();
        preimage.extend_from_slice(canonical);
        assert_eq!(app_digest_of(canonical), ContentRef::of(&preimage).0);
        // Full 32-byte digest, domain-separated from the `app_ref` preimage tag.
        assert_eq!(app_digest_of(canonical).len(), 32);
        assert_ne!(APP_DIGEST_DOMAIN, b"kx-app\0".as_slice());
    }
}
