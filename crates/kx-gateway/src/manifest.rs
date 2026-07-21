//! POC-6 (agentic creation) — the DYNAMIC project manifest: the model-proposed set
//! of files a scaffold authors for a use-case-specific project.
//!
//! The `app-manifest-plan` recipe's committed answer is a strict-JSON manifest —
//! UNTRUSTED model output. [`decode_manifest`] turns it into a validated file list
//! with the exact fail-closed discipline of [`kx_planner::decode_plan`] (IMP-5):
//! size-check BEFORE parse, strip a leading reasoning block / code fence, decode
//! into fixed flat structs (`deny_unknown_fields`, never a dynamic `serde_json::Value`
//! — so no float/NaN/unbounded-recursion path), then enforce the envelope invariants
//! (version, count, per-field size, safe relative paths, uniqueness, the reserved
//! marker). Total + panic-free over arbitrary bytes.
//!
//! The JSON decode lives in the HOST (kx-gateway), NOT gateway-core: gateway-core's
//! library deliberately never links a JSON manifest view (its `serde_json` is
//! dev-only). gateway-core owns the seam types + the marker-path const; the host
//! owns the decode.

use kx_gateway_core::MANIFEST_MARKER_PATH;
use serde::Deserialize;

/// POC-6: the DYNAMIC project-manifest contract — the system prompt teaching the
/// model to plan the SEPARATED SOURCE tree of a production-grade web app and emit EXACTLY one
/// strict `{"manifest":{"version":1,"files":[{"path","role"}]}}` envelope, the minimal surface
/// [`decode_manifest`] accepts (fail-closed, `deny_unknown_fields`, relative paths only).
///
/// This drives the HOSTED (Experience) lane. The build tooling (`package.json`, tsconfig, the
/// bundler config, the HTML + app entry point) is TEMPLATE-OWNED — the model must NOT plan it —
/// so the guaranteed-valid config always installs/builds/serves; the planner owns only the
/// per-goal SOURCE tree. The mandate forces real separation (components in their own files, a
/// stylesheet the component imports, no single-file monolith). `manifest_plan_directive` appends
/// a per-framework contract naming exactly what is provided vs what the model must emit. The
/// exact source-only envelope round-trips through [`decode_manifest`] (pinned by
/// `manifest_plan_contract_decodes_via_the_enforcer`). It lives beside the decoder — one place
/// owns the whole manifest contract (prompt + enforcer + coherence test) — because the scaffold
/// orchestrator that binds it is compiled unconditionally, while `prompt_library` is
/// `serve-engine`-gated.
pub(crate) const MANIFEST_PLAN_SYSTEM: &str = "You are a senior front-end engineer planning the \
SOURCE files of a COMPLETE, production-grade web app for a goal. The build tooling — \
`package.json`, the bundler + `tsconfig`, the HTML entry, and the app entry point — is ALREADY \
PROVIDED; do NOT plan any of them. Plan a well-SEPARATED source tree: the main component/page, \
focused child components each in their OWN file under a components directory, a stylesheet the \
main component imports, small hooks/helpers/types modules as needed, and at least one test. \
Separation is mandatory: NEVER put the whole app in one file; each component lives in its own \
file and is imported where used; styles live in a stylesheet the component imports (NOT inline \
style objects and NOT one giant inline style block); shared types/data go in their own module; no \
file re-declares what another file already exports. For EACH file write a short ROLE: one concrete \
sentence describing what that file contains and which siblings it imports. Plan a focused, \
coherent set — typically 4 to 10 source files; add a file only when it does distinct work. Reply \
with EXACTLY one JSON object and NOTHING else — no prose, no code fence, no explanation:\n\
{\"manifest\":{\"version\":1,\"files\":[{\"path\":\"<relative path>\",\"role\":\"<what this file \
contains>\"}]}}\n\
Rules: version is always 1; every path is RELATIVE (no leading slash, no `..` segment); do NOT \
include a content/body/language field or any field other than `path` and `role` per file; do NOT \
plan a `.kortecx/` path (reserved); do NOT plan `package.json`, a `tsconfig`, a bundler/build \
config, or the HTML / app entry point (all provided). Keep it minimal but complete.";

/// The per-framework CONTRACT appended to the planner directive: names exactly the
/// template-owned files the model must NOT emit (and what the entry imports), and the source
/// files it MUST emit, so the planned tree slots into the fixed framework scaffold. Unknown /
/// `"auto"` resolves to Vite-React (the template's own fallback).
fn framework_contract(framework: &str) -> &'static str {
    match framework {
        "next_js" => {
            "Framework: Next.js (App Router) + TypeScript. PROVIDED, do NOT emit: \
package.json, next.config.mjs, tsconfig.json, next-env.d.ts, app/layout.tsx (renders children and \
imports app/globals.css), app/globals.css. You MUST emit app/page.tsx as the default-export page \
component; put child components under app/components/ (one per file), a stylesheet the page \
imports (e.g. app/page.module.css), and a test app/page.test.tsx. Use only `next` and `react` — \
no other npm dependencies."
        }
        "svelte" => {
            "Framework: Svelte + TypeScript (Vite). PROVIDED, do NOT emit: package.json, \
svelte.config.js, vite.config.ts, tsconfig.json, index.html, src/vite-env.d.ts, src/main.ts \
(imports ./App.svelte and ./app.css), src/app.css. You MUST emit src/App.svelte as the root \
component; put child components under src/lib/ (one per file) and import them into App.svelte; \
each component's styles go in its own <style> block; add a test src/App.test.ts. Use only \
`svelte` — no other npm dependencies."
        }
        _ => {
            "Framework: Vite + React + TypeScript. PROVIDED, do NOT emit: package.json, \
vite.config.ts, tsconfig.json, index.html, src/main.tsx (renders the default export of ./App and \
imports ./index.css), src/index.css. You MUST emit src/App.tsx as the default-export root \
component that does `import './App.css'`; put child components under src/components/ (one per \
file), the src/App.css it imports, any src/hooks or src/types modules you need, and a test \
src/App.test.tsx. Use only `react` — no other npm dependencies."
        }
    }
}

/// Build the manifest-planner directive for an app `goal` on `framework`: the source-only
/// separation contract ([`MANIFEST_PLAN_SYSTEM`]) + the framework contract + the goal, passed as
/// the bound `prompt` DATA arg to the `app-manifest-plan` recipe (the scaffold-write precedent —
/// the directive is data, never an identity axis). The committed answer is decoded fail-closed by
/// [`decode_manifest`].
pub(crate) fn manifest_plan_directive(goal: &str, framework: &str) -> String {
    format!(
        "{MANIFEST_PLAN_SYSTEM}\n\n{}\n\nApp goal: {}",
        framework_contract(framework),
        goal.trim()
    )
}

/// Hard cap on the number of files a manifest may declare — a DoS bound
/// independent of the byte cap. Generous for a full runnable project, bounded for
/// the write loop.
pub(crate) const MAX_MANIFEST_FILES: usize = 48;

/// Hard cap on a manifest byte payload BEFORE parse — a hostile model cannot force
/// a large parse allocation past this.
pub(crate) const MAX_MANIFEST_BYTES: usize = 16 * 1024;

/// Per-path cap (defense-in-depth on the flat strings).
const MAX_MANIFEST_PATH_BYTES: usize = 160;
/// Per-role cap (defense-in-depth on the flat strings).
const MAX_MANIFEST_ROLE_BYTES: usize = 400;

/// One planned manifest file — a model-CHOSEN relative path + a short authoring
/// role the scaffold write step fills. Owned `String`s (vs `kx_gateway_core::ScaffoldFile`'s
/// `&'static str`) because the model chooses them at run time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ManifestFile {
    /// The relative project path (validated: relative, no `..`, safe charset).
    pub(crate) path: String,
    /// A short authoring role woven into the write step's prompt.
    pub(crate) role: String,
}

/// A fail-closed manifest-decode refusal (a closed vocabulary so tests assert the
/// exact reason). Mirrors `kx_planner::PlanError`'s shape.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ManifestError {
    /// The payload exceeded [`MAX_MANIFEST_BYTES`] before parsing.
    #[error("manifest is oversize: {got} bytes > max {max}")]
    Oversize {
        /// The payload byte length.
        got: usize,
        /// The cap.
        max: usize,
    },
    /// The payload was not valid UTF-8.
    #[error("manifest was not valid UTF-8")]
    NotUtf8,
    /// The strict envelope parse failed (non-JSON / wrong shape / unknown key).
    #[error("manifest is malformed: {diagnostic}")]
    Malformed {
        /// The serde diagnostic.
        diagnostic: String,
    },
    /// The version field was not the supported `1`.
    #[error("manifest version {version} is unsupported (expected 1)")]
    UnknownVersion {
        /// The declared version.
        version: u32,
    },
    /// The manifest declared no files.
    #[error("manifest declares no files")]
    Empty,
    /// The manifest declared more than [`MAX_MANIFEST_FILES`] files.
    #[error("manifest declares too many files: {got} > max {max}")]
    TooManyFiles {
        /// The declared count.
        got: usize,
        /// The cap.
        max: usize,
    },
    /// A declared path was not a safe relative project path (or was the reserved
    /// manifest marker).
    #[error("manifest path is invalid: {path}")]
    InvalidPath {
        /// The offending path.
        path: String,
    },
    /// A path or role field exceeded its per-field cap.
    #[error("manifest field too long ({what}): {got} bytes > max {max}")]
    FieldTooLong {
        /// Which field (`path` / `role`).
        what: &'static str,
        /// The field byte length.
        got: usize,
        /// The cap.
        max: usize,
    },
    /// Two files declared the same path.
    #[error("manifest declares a duplicate path: {path}")]
    DuplicatePath {
        /// The duplicated path.
        path: String,
    },
}

/// The strict manifest envelope — `{"manifest":{"version":1,"files":[…]}}`. Flat
/// structs only (strings + `u32`), `deny_unknown_fields` on each, so no dynamic
/// `serde_json::Value` / float / unbounded-recursion path exists.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestEnvelope {
    manifest: WireManifest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireManifest {
    version: u32,
    files: Vec<WireManifestFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireManifestFile {
    path: String,
    role: String,
}

/// `true` iff `p` is a safe RELATIVE project path: non-empty, ≤ cap, no leading /
/// trailing `/`, no `.`/`..`/empty segment, and every byte in the portable set
/// `[A-Za-z0-9._/-]`. Fail-closed — the model chose it (untrusted).
fn is_safe_manifest_path(p: &str) -> bool {
    if p.is_empty() || p.len() > MAX_MANIFEST_PATH_BYTES {
        return false;
    }
    if p.starts_with('/') || p.ends_with('/') {
        return false;
    }
    if !p
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
    {
        return false;
    }
    p.split('/')
        .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// Strip a SINGLE leading reasoning block (`<think>…</think>` / `<|channel>…<channel|>`)
/// then a surrounding markdown code fence, so the strict parser sees bare `{…}`.
/// Total + panic-free (the delimiters are ASCII). A local copy of the `kx-planner`
/// decoder's `extract_json_envelope`.
fn strip_json_wrappers(text: &str) -> &str {
    let mut t = text.trim();
    for (open, close) in [("<think>", "</think>"), ("<|channel>", "<channel|>")] {
        if let Some(rest) = t.strip_prefix(open) {
            t = match rest.find(close) {
                Some(i) => rest[i + close.len()..].trim(),
                None => "",
            };
            break;
        }
    }
    let Some(rest) = t.strip_prefix("```") else {
        return t;
    };
    let inner = match rest.find('\n') {
        Some(nl) => &rest[nl + 1..],
        None => rest,
    };
    match inner.rfind("```") {
        Some(i) => inner[..i].trim(),
        None => inner.trim(),
    }
}

/// Decode a model-proposed project manifest, fail-closed (see the module doc).
/// Total + panic-free over arbitrary `bytes`.
pub(crate) fn decode_manifest(bytes: &[u8]) -> Result<Vec<ManifestFile>, ManifestError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::Oversize {
            got: bytes.len(),
            max: MAX_MANIFEST_BYTES,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ManifestError::NotUtf8)?;
    let stripped = strip_json_wrappers(text);
    let envelope: ManifestEnvelope =
        serde_json::from_str(stripped).map_err(|e| ManifestError::Malformed {
            diagnostic: e.to_string(),
        })?;
    let m = envelope.manifest;
    if m.version != 1 {
        return Err(ManifestError::UnknownVersion { version: m.version });
    }
    if m.files.is_empty() {
        return Err(ManifestError::Empty);
    }
    if m.files.len() > MAX_MANIFEST_FILES {
        return Err(ManifestError::TooManyFiles {
            got: m.files.len(),
            max: MAX_MANIFEST_FILES,
        });
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(m.files.len());
    for f in m.files {
        if f.role.len() > MAX_MANIFEST_ROLE_BYTES {
            return Err(ManifestError::FieldTooLong {
                what: "role",
                got: f.role.len(),
                max: MAX_MANIFEST_ROLE_BYTES,
            });
        }
        // The marker is reserved (an internal artifact, never a planned file).
        if f.path == MANIFEST_MARKER_PATH || !is_safe_manifest_path(&f.path) {
            return Err(ManifestError::InvalidPath { path: f.path });
        }
        if !seen.insert(f.path.clone()) {
            return Err(ManifestError::DuplicatePath { path: f.path });
        }
        out.push(ManifestFile {
            path: f.path,
            role: f.role,
        });
    }
    Ok(out)
}

/// Serialize a file set back into the strict `{"manifest":{"version":1,"files":[…]}}`
/// envelope, so the scaffold can persist a SERVER-authored plan as the durable
/// `.kortecx/manifest.json` marker — not only a model-authored one.
///
/// Why this exists: the marker is what `GetScaffoldStatus` reads to report the planned
/// set. Two lanes reach a plan the model did not write — the hosted lane's
/// authored-files fallback (no served model / a decode failure) and the scheduled
/// lane's preserved base skeleton — and before this, neither could persist one, so
/// `status()` fell through to the scheduled skeleton and the console showed the wrong
/// tree until the write loop happened to overwrite it.
///
/// The output is the SAME envelope [`decode_manifest`] accepts, deliberately: the
/// marker has exactly one format regardless of who authored it, so a resume cannot
/// tell the difference and no second parser exists to drift.
///
/// # Errors
/// Returns [`ManifestError`] if the set would not survive its own decoder — empty,
/// over [`MAX_MANIFEST_FILES`], a reserved/unsafe path, an over-cap field, or a
/// duplicate. Encoding is validated by round-trip rather than trusted: a caller that
/// hands in something the decoder would reject learns here, not on resume.
pub(crate) fn encode_manifest(files: &[ManifestFile]) -> Result<Vec<u8>, ManifestError> {
    let escape = |s: &str| -> String {
        // The validated charset for paths excludes every JSON metacharacter, but roles
        // are free text (a fallback role is ours, a planned role is the model's), so
        // escape both rather than assume.
        let mut out = String::with_capacity(s.len() + 8);
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    use std::fmt::Write as _;
                    let _ = write!(out, "\\u{:04x}", c as u32);
                }
                c => out.push(c),
            }
        }
        out
    };
    let body: Vec<String> = files
        .iter()
        .map(|f| {
            format!(
                "{{\"path\":\"{}\",\"role\":\"{}\"}}",
                escape(&f.path),
                escape(&f.role)
            )
        })
        .collect();
    let json = format!(
        "{{\"manifest\":{{\"version\":1,\"files\":[{}]}}}}",
        body.join(",")
    );
    let bytes = json.into_bytes();
    // Round-trip through the real enforcer: the marker we persist must be one the
    // resume path can read back. This also gives every caller the decoder's fail-closed
    // vocabulary for free instead of a second, weaker set of checks here.
    decode_manifest(&bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_manifest_round_trips_through_the_enforcer() {
        let files = vec![
            ManifestFile {
                path: "src/App.tsx".to_string(),
                role: "the root component".to_string(),
            },
            ManifestFile {
                path: "README.md".to_string(),
                // Quotes + a newline: a role is free text, so the escaper is load-bearing.
                role: "what \"this\" app does\nand how to run it".to_string(),
            },
        ];
        let bytes = encode_manifest(&files).expect("encodes");
        assert_eq!(decode_manifest(&bytes).expect("decodes"), files);
    }

    #[test]
    fn encode_manifest_refuses_what_its_own_decoder_would_reject() {
        // Empty: the decoder's `Empty` refusal must surface at encode time, not on resume.
        assert_eq!(encode_manifest(&[]).unwrap_err(), ManifestError::Empty);
        // The reserved marker path can never be a planned file, even server-authored.
        let reserved = vec![ManifestFile {
            path: MANIFEST_MARKER_PATH.to_string(),
            role: "the marker".to_string(),
        }];
        assert_eq!(
            encode_manifest(&reserved).unwrap_err(),
            ManifestError::InvalidPath {
                path: MANIFEST_MARKER_PATH.to_string()
            }
        );
    }

    #[test]
    fn decode_manifest_accepts_a_wrapped_dynamic_project() {
        // Gemma-style: a leading reasoning block + a ```json fence around the JSON.
        let raw = "<think>plan a vite react app</think>\n```json\n{\"manifest\":{\"version\":1,\
                   \"files\":[{\"path\":\"package.json\",\"role\":\"the npm manifest\"},\
                   {\"path\":\"src/App.tsx\",\"role\":\"the root component\"}]}}\n```";
        let files = decode_manifest(raw.as_bytes()).expect("wrapped manifest decodes");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "package.json");
        assert_eq!(files[1].path, "src/App.tsx");
        assert_eq!(files[1].role, "the root component");
    }

    #[test]
    fn decode_manifest_is_fail_closed() {
        // Not JSON.
        assert!(matches!(
            decode_manifest(b"not a manifest"),
            Err(ManifestError::Malformed { .. })
        ));
        // Unknown key (deny_unknown_fields) — closes the "smuggle an extra field" vector.
        let extra = "{\"manifest\":{\"version\":1,\"files\":[{\"path\":\"a.txt\",\"role\":\"r\",\
                     \"model\":\"gemma\"}]}}";
        assert!(matches!(
            decode_manifest(extra.as_bytes()),
            Err(ManifestError::Malformed { .. })
        ));
        // Wrong version.
        let v2 = "{\"manifest\":{\"version\":2,\"files\":[{\"path\":\"a.txt\",\"role\":\"r\"}]}}";
        assert!(matches!(
            decode_manifest(v2.as_bytes()),
            Err(ManifestError::UnknownVersion { version: 2 })
        ));
        // Empty file set.
        let empty = "{\"manifest\":{\"version\":1,\"files\":[]}}";
        assert_eq!(decode_manifest(empty.as_bytes()), Err(ManifestError::Empty));
        // Path traversal.
        let esc = "{\"manifest\":{\"version\":1,\"files\":[{\"path\":\"../etc/passwd\",\"role\":\"r\"}]}}";
        assert!(matches!(
            decode_manifest(esc.as_bytes()),
            Err(ManifestError::InvalidPath { .. })
        ));
        // Absolute path.
        let abs = "{\"manifest\":{\"version\":1,\"files\":[{\"path\":\"/tmp/x\",\"role\":\"r\"}]}}";
        assert!(matches!(
            decode_manifest(abs.as_bytes()),
            Err(ManifestError::InvalidPath { .. })
        ));
        // The reserved marker path.
        let marker = format!(
            "{{\"manifest\":{{\"version\":1,\"files\":[{{\"path\":\"{MANIFEST_MARKER_PATH}\",\"role\":\"r\"}}]}}}}"
        );
        assert!(matches!(
            decode_manifest(marker.as_bytes()),
            Err(ManifestError::InvalidPath { .. })
        ));
        // Duplicate path.
        let dup = "{\"manifest\":{\"version\":1,\"files\":[{\"path\":\"a.txt\",\"role\":\"r\"},\
                   {\"path\":\"a.txt\",\"role\":\"r2\"}]}}";
        assert!(matches!(
            decode_manifest(dup.as_bytes()),
            Err(ManifestError::DuplicatePath { .. })
        ));
        // Oversize (before parse).
        let big = vec![b'x'; MAX_MANIFEST_BYTES + 1];
        assert!(matches!(
            decode_manifest(&big),
            Err(ManifestError::Oversize { .. })
        ));
    }

    #[test]
    fn decode_manifest_caps_the_file_count() {
        use std::fmt::Write as _;
        let mut files = String::new();
        for i in 0..=MAX_MANIFEST_FILES {
            if i > 0 {
                files.push(',');
            }
            let _ = write!(files, "{{\"path\":\"f{i}.txt\",\"role\":\"r\"}}");
        }
        let over = format!("{{\"manifest\":{{\"version\":1,\"files\":[{files}]}}}}");
        assert!(matches!(
            decode_manifest(over.as_bytes()),
            Err(ManifestError::TooManyFiles { .. })
        ));
    }

    /// The canonical SOURCE-ONLY manifest the contract teaches MUST decode through the same
    /// fail-closed enforcer the runtime uses ([`decode_manifest`]) — the render↔enforce
    /// coherence guard (mirrors the planner example test in `prompt_library`). The example is
    /// a SEPARATED tree (App + a component + a stylesheet + types + a test), and it carries NO
    /// template-owned config (package.json / entry) — POC-6 makes those template-owned.
    #[test]
    fn manifest_plan_contract_decodes_via_the_enforcer() {
        const EXAMPLE: &str = "{\"manifest\":{\"version\":1,\"files\":[\
{\"path\":\"src/App.tsx\",\"role\":\"the default-export root component; imports ./App.css and \
components from ./components/\"},\
{\"path\":\"src/App.css\",\"role\":\"the styles imported by App.tsx\"},\
{\"path\":\"src/components/Card.tsx\",\"role\":\"a card component imported by App.tsx\"},\
{\"path\":\"src/types.ts\",\"role\":\"shared TypeScript types\"},\
{\"path\":\"src/App.test.tsx\",\"role\":\"a smoke test rendering App\"},\
{\"path\":\"README.md\",\"role\":\"how to run the project\"}]}}";
        let files = decode_manifest(EXAMPLE.as_bytes())
            .expect("the taught manifest example must decode via the runtime enforcer");
        assert!(
            files.len() >= 5,
            "the example teaches a separated source tree"
        );
        assert!(files.iter().any(|f| f.path == "src/App.tsx"));
        assert!(files.iter().any(|f| f.path == "src/components/Card.tsx"));
        // The tooling is template-owned — the contract must NOT teach config paths.
        assert!(!files.iter().any(|f| f.path == "package.json"));
    }

    #[test]
    fn manifest_plan_directive_is_framework_aware_source_only() {
        let d = manifest_plan_directive("a kanban board with drag and drop", "vite_react");
        assert!(d.contains("a kanban board with drag and drop"));
        assert!(d.contains("\"manifest\""));
        assert!(d.contains("\"version\":1"));
        assert!(d.contains("EXACTLY one JSON object"));
        // Framework contract present + config forbidden.
        assert!(d.contains("Vite + React"));
        assert!(d.contains("do NOT emit"));
        assert!(d.contains("src/App.tsx"));
        // Next / Svelte contracts route distinctly.
        assert!(manifest_plan_directive("x", "next_js").contains("app/page.tsx"));
        assert!(manifest_plan_directive("x", "svelte").contains("src/App.svelte"));
    }
}
