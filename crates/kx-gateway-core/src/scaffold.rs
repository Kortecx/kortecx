//! POC-5a — the App-scaffold seam + the fixed skeleton + pure helpers.
//!
//! `ScaffoldApp` drives a NET-NEW agentic loop that writes a FIXED skeleton of
//! agentic-app files into a fresh CoW-on-CAS branch: for each skeleton file, a
//! single Pure greedy model step (the proven `react-edit` pattern) authors the file
//! body, which is content-addressed and `AdvanceBranch`-ed into the manifest. The
//! STRUCTURE is fixed (deterministic + testable); the CONTENT is model-authored.
//!
//! The ORCHESTRATION (spawn a background task, await each step, advance the branch)
//! lives in the HOST behind the [`AppScaffolder`] seam — gateway-core is a read-fold
//! and propose-proxy that deliberately owns no runtime/spawn surface (the AppCatalog
//! and BranchStore precedent). gateway-core provides the seam, the fixed skeleton, the
//! pure prompt/body helpers, and a NO-tokio single-shot commit check
//! ([`try_committed_body`]) that the host's await loop polls.

use kx_content::ContentRef;
use kx_mote::MoteId;
use kx_projection::MoteState;

use crate::error::{internal, GatewayError};
use crate::reader::{ContentReader, JournalReader};

/// The recipe handle the host seeds for the scaffold write step (a single Pure
/// greedy model step, the `react-edit` clone). The host's `provision.rs` MUST seed
/// this exact handle (the scaffold contract).
pub const APP_SCAFFOLD_WRITE_RECIPE_HANDLE: &str = "kx/recipes/app-scaffold-write";

/// POC-6 (agentic creation): the recipe handle the host seeds for the DYNAMIC
/// manifest planner — a single Pure greedy model step whose committed answer is a
/// strict-JSON project manifest (`{"manifest":{"version":1,"files":[…]}}`). The
/// host's `provision.rs` MUST seed this exact handle (seeded only when a model is
/// served). The scaffold orchestrator binds it once per creation to plan the
/// use-case-specific file set, then drives the write loop over that set.
pub const APP_MANIFEST_PLAN_RECIPE_HANDLE: &str = "kx/recipes/app-manifest-plan";

/// POC-6: the durable in-branch record of the planned dynamic file set. Written
/// (as the planner's committed JSON) once the manifest lands, so `GetScaffoldStatus`
/// and a RESUMED run read the SAME planned paths the model chose — a resume never
/// re-plans (the committed plan is the truth). Excluded from the reported
/// file-progress set (an internal marker, never a "project file").
pub const MANIFEST_MARKER_PATH: &str = ".kortecx/manifest.json";

/// One fixed skeleton file: a stable path + the authoring role the model fills.
pub struct ScaffoldFile {
    /// The manifest path (stable — the deterministic e2e asserts exactly these).
    pub path: &'static str,
    /// A short role description woven into the authoring prompt.
    pub role: &'static str,
}

/// The BASE skeleton of a scaffolded agentic-app project — the files every scheduled App
/// has, whose CONTENT the model authors. Ordered so earlier files (README, app.json)
/// become coherence context for later ones.
///
/// PRESERVED, NOT EXHAUSTIVE. The scheduled lane writes this set **plus** the use-case
/// files its manifest planner adds on top (see the host's `resolve_manifest_scheduled`),
/// so a scaffolded project is a superset of this list. These five stay inviolable because
/// downstream surfaces assume `app.json` and `prompts/system.md` exist; the planner may
/// only ADD, and a plan that re-declares one of these paths has it dropped.
///
/// `skeleton_paths_are_stable` pins this CONSTANT, which is the right thing to pin. An
/// earlier version of this comment claimed "the e2e asserts exactly these paths" — the
/// witness it meant asserts CONTAINMENT, and is `#[ignore]` + `cfg(feature = "inference")`
/// so it never runs in CI at all. Do not treat that comment's successor as a licence to
/// assume the produced set equals this one.
pub const SKELETON: &[ScaffoldFile] = &[
    ScaffoldFile {
        path: "README.md",
        role: "a concise README: what the app does, how to run it, and its inputs/outputs",
    },
    ScaffoldFile {
        path: "app.json",
        role: "a JSON object describing the app: name, a one-line description, the model \
               route intent, and the high-level steps (a plain JSON object, no comments)",
    },
    ScaffoldFile {
        path: "prompts/system.md",
        role: "the system prompt that steers the agent's behaviour for this app",
    },
    ScaffoldFile {
        path: "rules/guardrails.md",
        role: "the guardrails / rules the agent must always follow (safety, scope, refusals)",
    },
    ScaffoldFile {
        path: "skills/main.md",
        role: "the primary skill: a focused instruction + the tools it may use, in markdown",
    },
];

/// The scaffold phase reported by `GetScaffoldStatus`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScaffoldPhase {
    /// The branch is being prepared / the first file is in flight.
    Planning,
    /// Skeleton files are being authored (files_pending non-empty).
    Writing,
    /// Every skeleton file has been advanced into the branch.
    Done,
    /// The orchestration failed (the `detail` carries the advisory reason).
    Failed,
}

/// The resolved scaffold status (the host derives `files_*` from the durable branch
/// manifest and `phase`/`detail` from its advisory tracker, falling back to the
/// manifest when untracked — e.g. after a restart).
#[derive(Clone, Debug)]
pub struct ScaffoldStatus {
    /// The current phase.
    pub phase: ScaffoldPhase,
    /// Skeleton paths already present in the branch manifest.
    pub files_done: Vec<String>,
    /// Skeleton paths still to write.
    pub files_pending: Vec<String>,
    /// Advisory progress / failure text (never authority).
    pub detail: String,
    /// POC-6: the project path currently being authored (streamed live), if any.
    /// `None` outside the write phase.
    pub writing_path: Option<String>,
    /// POC-6: the run instance streaming the writing file's tokens — 16 bytes.
    /// The WS `/tokens` ownership gate; surfaced as hex on the wire. `None` when
    /// no file is being written.
    pub writing_instance_id: Option<[u8; 16]>,
    /// POC-6: the write mote whose decode streams the writing file — 32 bytes.
    /// The token-broker key; surfaced as hex on the wire. `None` when no file is
    /// being written.
    pub writing_mote_id: Option<[u8; 32]>,
}

/// The host-side App-scaffold orchestrator seam. The host impl owns the runtime
/// (spawns the background task, runs the await loop, holds the advisory tracker +
/// the binder/submitter/branches/locks seams). A `None` seam ⇒ `ScaffoldApp` /
/// `GetScaffoldStatus` return `unimplemented` (no served model / branch store).
pub trait AppScaffolder: Send + Sync {
    /// Start (or resume) the background scaffold of `branch_handle` toward `goal`.
    /// Returns `resumed` (true iff the branch already held ≥1 skeleton file). Spawns
    /// a background task and returns immediately (the propose-proxy contract).
    fn start(&self, principal: &str, branch_handle: &str, goal: &str)
        -> Result<bool, GatewayError>;

    /// D213 Experience lane: start (or resume) a HOSTED-app scaffold — author the
    /// framework template's model-authored files (the visible page + README) into
    /// `branch_handle` toward `goal`. `envelope_json` is the app's opaque canonical
    /// envelope (the host parses the framework from it — gateway-core keeps app bytes
    /// opaque). The static config files are template-owned (written to disk by the
    /// hosted-app supervisor), so only the authored files are scaffolded here. Default
    /// impl: `Unsupported` (a scaffolder that cannot author a model file).
    fn start_hosted(
        &self,
        _principal: &str,
        _branch_handle: &str,
        _envelope_json: &[u8],
        _goal: &str,
    ) -> Result<bool, GatewayError> {
        Err(GatewayError::FailedPrecondition(
            "hosted-app scaffold is not supported by this scaffolder",
        ))
    }

    /// The current scaffold status for a branch (caller-scoped via the host's
    /// branch store).
    fn status(&self, principal: &str, branch_handle: &str) -> Result<ScaffoldStatus, GatewayError>;
}

/// The outcome of one single-shot terminal-commit check (the host's await loop
/// polls this with its own sleep — gateway-core owns no timer).
pub enum ScaffoldStep {
    /// Not yet committed (the host keeps polling until its deadline).
    Pending,
    /// Committed — the body + its content ref (advance the branch to this ref).
    Ready {
        /// The committed body's 32-byte content ref.
        result_ref: [u8; 32],
        /// The committed body bytes (the host checks emptiness, GR15).
        body: Vec<u8>,
    },
    /// The step reached a non-committed terminal (Failed/Repudiated/Inconsistent).
    Failed,
}

/// Single-shot, NO-tokio check of a write step's terminal mote: fold the read-only
/// projection once and resolve the committed body (mirrors the `GetContent` fold).
/// The host's await loop calls this between sleeps.
pub fn try_committed_body(
    reader: &dyn JournalReader,
    content: &dyn ContentReader,
    terminal: MoteId,
) -> Result<ScaffoldStep, GatewayError> {
    let head = reader.current_seq().map_err(internal)?;
    let (projection, _) = crate::view::fold_through(reader, head)?;
    // Bind the state into an owned value (both fields are `Copy`) so the iterator's
    // borrow of `projection` ends before we re-borrow it for `result_ref_of`.
    let state = projection
        .iter_motes()
        .find(|(id, _)| *id == terminal)
        .map(|(_, s)| s);
    match state {
        Some(MoteState::Committed) => {
            let result_ref = projection
                .result_ref_of(&terminal)
                .ok_or_else(|| internal("committed scaffold step has no result ref"))?;
            let body = content
                .get(&ContentRef::from_bytes(result_ref.0))
                .ok_or_else(|| internal("committed scaffold body missing from store"))?;
            Ok(ScaffoldStep::Ready {
                result_ref: result_ref.0,
                body,
            })
        }
        Some(MoteState::Failed | MoteState::Repudiated | MoteState::Inconsistent) => {
            Ok(ScaffoldStep::Failed)
        }
        // None (not yet folded) or Pending/Scheduled ⇒ keep polling.
        _ => Ok(ScaffoldStep::Pending),
    }
}

/// A human label for the framework a hosted-app file belongs to (drives the authoring
/// prompt's framing). `None` is the agentic (scheduled) lane; `Some(fw)` is a hosted
/// (Experience) framework project. Unknown / `"auto"` labels resolve to Vite-React.
#[must_use]
fn framework_label(framework: &str) -> &'static str {
    match framework {
        "next_js" => "a production-grade Next.js (App Router) + TypeScript web project",
        "svelte" => "a production-grade Svelte + TypeScript (Vite) web project",
        _ => "a production-grade Vite + React + TypeScript web project",
    }
}

/// The maximum bytes one sibling's distilled API summary may contribute to the next
/// file's authoring prompt. Signatures are tiny by design (a name list + prop shape), so
/// this only guards against a pathological `*Props` block; unlike a raw sibling BODY, an
/// API summary this small can be carried for EVERY prior sibling without approaching the
/// `n_tokens_all <= n_batch` decode limit.
const MAX_SIBLING_API_BYTES: usize = 360;

/// `true` for a path whose body is TypeScript/JavaScript source (the hosted lane), whose
/// export surface and prop types [`distill_module_api`] can summarize. The scheduled lane's
/// markdown/JSON files return `false` (nothing to import).
fn is_ts_source(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "mts" | "cts")
    )
}

fn push_unique(out: &mut Vec<String>, name: &str) {
    let n = name.trim();
    if !n.is_empty() && !out.iter().any(|e| e == n) {
        out.push(n.to_string());
    }
}

/// The names a TS/JS module `export`s, in first-seen (deterministic) order. A line scanner,
/// not a parser: it recognizes the shipped export forms (`export const/function/class/type/
/// interface/enum NAME`, `export default …`, and `export { a, b as c } [from …]`). Anything
/// exotic it misses is caught by the serve-time `tsc --noEmit` gate, never by a failed scaffold.
fn exported_symbols(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.lines() {
        let Some(rest) = raw.trim_start().strip_prefix("export ") else {
            continue;
        };
        let rest = rest.trim_start();
        // `export { a, b as c }` (optionally `from '…'`) — an alias exports the alias name.
        if let Some(inner) = rest.strip_prefix('{') {
            let inner = inner.split('}').next().unwrap_or(inner);
            for item in inner.split(',') {
                let item = item.trim();
                if item.is_empty() {
                    continue;
                }
                let name = item.rsplit(" as ").next().unwrap_or(item).trim();
                push_unique(&mut out, name);
            }
            continue;
        }
        // `export default …` — the module has a default export (imported under any name).
        if rest
            .strip_prefix("default")
            .is_some_and(|a| a.is_empty() || a.starts_with(|c: char| c.is_whitespace()))
        {
            push_unique(&mut out, "default");
            continue;
        }
        // `export [async] <kw> NAME …`
        let after = rest.strip_prefix("async ").unwrap_or(rest);
        for kw in [
            "function ",
            "const enum ",
            "const ",
            "let ",
            "var ",
            "abstract class ",
            "class ",
            "type ",
            "interface ",
            "enum ",
        ] {
            if let Some(tail) = after.strip_prefix(kw) {
                let name = tail
                    .trim_start()
                    .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
                    .next()
                    .unwrap_or("");
                push_unique(&mut out, name);
                break;
            }
        }
    }
    out
}

/// Return the substring INSIDE the brace block that opens at `open` (a byte offset of a `{`),
/// up to its matching `}`. Empty if unbalanced or `open` is not a `{`.
fn balanced_block(text: &str, open: usize) -> &str {
    let bytes = text.as_bytes();
    if bytes.get(open) != Some(&b'{') {
        return "";
    }
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[open + 1..i];
                }
            }
            _ => {}
        }
    }
    ""
}

/// Compact `field: Type` entries from a props-block body (the text between its braces). Takes
/// the leading `name[?]: …` of each `;`/newline-separated segment, skipping methods/nested junk.
fn field_entries(block: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for seg in block.split([';', '\n']) {
        let seg = seg.trim().trim_end_matches(',').trim();
        if seg.is_empty() || seg.starts_with("//") {
            continue;
        }
        let Some(colon) = seg.find(':') else {
            continue;
        };
        let name = seg[..colon].trim().trim_end_matches('?');
        // A field name is a bare identifier; a `(` before the colon is a method — skip.
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        {
            continue;
        }
        let mut entry = seg.to_string();
        if entry.len() > 60 {
            entry.truncate(60);
            entry.push('…');
        }
        if !out.iter().any(|e| e == &entry) {
            out.push(entry);
        }
    }
    out
}

/// The prop shapes a `.tsx`/`.jsx` module declares — each `<Name>Props { field: Type; … }` —
/// so a parent renders the component with EXACTLY the props it accepts. The single most
/// common hosted-scaffold break is a parent passing flat props to a child that declared one
/// object; naming the child's prop shape in the parent's prompt closes it.
fn prop_shapes(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (idx, _) in text.match_indices("Props") {
        let line_start = text[..idx].rfind('\n').map_or(0, |p| p + 1);
        let head = text[line_start..idx].trim_start();
        let head = head.strip_prefix("export ").unwrap_or(head);
        if !(head.starts_with("interface ") || head.starts_with("type ")) {
            continue;
        }
        // The declared name is the identifier run ending in this `Props`.
        let name_start = text[..idx]
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
            .map_or(0, |p| p + 1);
        let name = &text[name_start..idx + "Props".len()];
        let Some(brace_rel) = text[idx..].find('{') else {
            continue;
        };
        let fields = field_entries(balanced_block(text, idx + brace_rel));
        if fields.is_empty() {
            continue;
        }
        let compact = format!("{name} {{ {} }}", fields.join("; "));
        if !out.iter().any(|e| e == &compact) {
            out.push(compact);
        }
    }
    out
}

/// The byte offset of the close matching the opener `oc` at `open` (which must be `oc`), or
/// `None` if unbalanced. Generic over `{}` / `()` / `<>`.
fn matching_close(text: &str, open: usize, oc: u8, cc: u8) -> Option<usize> {
    let b = text.as_bytes();
    if b.get(open) != Some(&oc) {
        return None;
    }
    let mut depth = 0i32;
    for (i, &ch) in b.iter().enumerate().skip(open) {
        if ch == oc {
            depth += 1;
        } else if ch == cc {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// The top-level type annotation of a parameter list body — the text after the FIRST `:` that
/// sits at brace/paren/bracket/angle depth 0 (so `{ a, b }: Props` → `Props`, `props: Props` →
/// `Props`, `{ a, b }` → `None`).
fn top_level_type(params: &str) -> Option<&str> {
    let b = params.as_bytes();
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'{' | b'(' | b'[' | b'<' => depth += 1,
            b'}' | b')' | b']' | b'>' => depth -= 1,
            b':' if depth == 0 => return Some(params[i + 1..].trim()),
            _ => {}
        }
    }
    None
}

/// A compact props descriptor for a component whose declaration text (everything after the
/// component name) is `after`: `props: T` for a typed/`React.FC<T>` component, `no props` for a
/// `React.FC` or `()` component, else the raw destructure (`props { a, b }`).
fn component_props(after: &str) -> String {
    for m in [
        "React.FC<",
        "React.FunctionComponent<",
        "FC<",
        "FunctionComponent<",
    ] {
        if let Some(i) = after.find(m) {
            let lt = i + m.len() - 1; // the `<`
            if let Some(gt) = matching_close(after, lt, b'<', b'>') {
                let t = after[lt + 1..gt].trim();
                if !t.is_empty() {
                    return format!("props: {t}");
                }
            }
        }
    }
    if after.contains("React.FC")
        || after.contains("React.FunctionComponent")
        || after.contains(": FC")
        || after.contains(": FunctionComponent")
    {
        return "no props".to_string(); // an FC with no generic ⇒ no props
    }
    if let Some(op) = after.find('(') {
        if let Some(cl) = matching_close(after, op, b'(', b')') {
            let params = after[op + 1..cl].trim();
            if params.is_empty() {
                return "no props".to_string();
            }
            if let Some(t) = top_level_type(params) {
                return format!("props: {t}");
            }
            let mut d = format!("props {params}");
            if d.len() > 60 {
                d.truncate(60);
                d.push('…');
            }
            return d;
        }
    }
    "no props".to_string()
}

/// The top-level keys of the FIRST `return { … }` at depth 1 of the function body opening at
/// `body_open` — a hook/util's return OBJECT shape. Ignores nested returns (e.g. a `useMemo`
/// callback), so `useX = () => { … return { a, b, c } }` yields `[a, b, c]`, not the inner one.
fn top_level_return_keys(text: &str, body_open: usize) -> Option<Vec<String>> {
    let b = text.as_bytes();
    if b.get(body_open) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let mut i = body_open;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return None;
                }
            }
            _ => {
                if depth == 1 && text[i..].starts_with("return") && !text[..i].is_empty() {
                    let prev = b[i - 1];
                    if !(prev.is_ascii_alphanumeric() || prev == b'_') {
                        let after = &text[i + "return".len()..];
                        let trimmed = after.trim_start();
                        if let Some(brace) = trimmed.strip_prefix('{') {
                            let _ = brace;
                            let brace_pos = i + "return".len() + (after.len() - trimmed.len());
                            let keys = object_keys(balanced_block(text, brace_pos));
                            if !keys.is_empty() {
                                return Some(keys);
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Top-level property names of an object-literal body (the text between its braces): the
/// identifier before each `:` or a shorthand key, at brace/paren/bracket depth 0.
fn object_keys(block: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for seg in split_top_level(block) {
        let seg = seg.trim();
        if seg.is_empty() || seg.starts_with("//") || seg.starts_with("...") {
            continue;
        }
        let key = seg.split(':').next().unwrap_or("").trim();
        if !key.is_empty()
            && key
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
            && !out.iter().any(|k| k == key)
        {
            out.push(key.to_string());
        }
    }
    out
}

/// Split a block body on top-level commas (respecting `{}`/`()`/`[]` nesting).
fn split_top_level(block: &str) -> Vec<&str> {
    let b = block.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => depth -= 1,
            b',' if depth == 0 => {
                out.push(&block[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&block[start..]);
    out
}

/// Rich per-export descriptors: a component's prop signature (`Foo(props: T)` / `Foo(no props)`)
/// and a hook/util's return OBJECT shape (`useFoo() returns { a, b }`). Falls back to the bare
/// name for a type/interface/class/enum/value. This is what closes the prop-shape and
/// return-shape drift a bare export-name list cannot: the entry sees exactly what to pass a child
/// and exactly what a hook returns.
fn describe_exports(text: &str, is_jsx: bool) -> Vec<String> {
    // Identifiers that are default-exported by name (`export default TipInput`), so a component
    // DEFINED as `const X = …` then default-exported gets a rich signature too — the common React
    // pattern the tip-calc scaffold used.
    let default_idents: Vec<String> = text
        .lines()
        .filter_map(|l| {
            let t = l.trim_start().strip_prefix("export default ")?;
            if t.starts_with("function")
                || t.starts_with("class")
                || t.starts_with('(')
                || t.starts_with('{')
            {
                return None;
            }
            let id: String = t
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                .collect();
            (!id.is_empty()).then_some(id)
        })
        .collect();

    let mut out: Vec<String> = Vec::new();
    let push = |s: String, out: &mut Vec<String>| {
        if !s.is_empty() && !out.iter().any(|e| e == &s) {
            out.push(s);
        }
    };
    for (line_start, raw) in line_offsets(text) {
        let trimmed = raw.trim_start();
        // Accept `export [default] [async] (const|function) NAME`, or a bare
        // `[async] (const|function) NAME` whose NAME is default-exported elsewhere.
        let (rest, is_export, mut is_default) = match trimmed.strip_prefix("export ") {
            Some(r) => {
                let d = r.trim_start().starts_with("default ");
                (
                    r.trim_start().strip_prefix("default ").unwrap_or(r),
                    true,
                    d,
                )
            }
            None => (trimmed, false, false),
        };
        let rest = rest.strip_prefix("async ").unwrap_or(rest);
        let after_kw = rest
            .strip_prefix("function ")
            .or_else(|| rest.strip_prefix("const "))
            .map(str::trim_start);
        let Some(after_kw) = after_kw else { continue };
        let name: String = after_kw
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
            .collect();
        if name.is_empty() {
            continue;
        }
        let default_by_name = default_idents.iter().any(|d| d == &name);
        if !is_export && !default_by_name {
            continue; // a private local, not part of the module's surface
        }
        is_default |= default_by_name;
        let name_abs = line_start + (raw.len() - after_kw.len());
        let after = &text[name_abs + name.len()..];
        let pascal = name.chars().next().is_some_and(char::is_uppercase);
        let looks_component = is_jsx && (pascal || after.contains("React.FC"));
        let label = if is_default {
            format!("default {name}")
        } else {
            name.clone()
        };
        if looks_component {
            push(format!("{label}({})", component_props(after)), &mut out);
        } else if let Some(bo) = after.find('{') {
            if let Some(keys) = top_level_return_keys(text, name_abs + name.len() + bo) {
                push(
                    format!("{label}() returns {{ {} }}", keys.join(", ")),
                    &mut out,
                );
            } else {
                push(label, &mut out);
            }
        } else {
            push(label, &mut out);
        }
    }
    out
}

/// `(line_start_byte_offset, line_text)` for each line — so a matched declaration can be located
/// back in the full `text` for brace scanning.
fn line_offsets(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for line in text.split_inclusive('\n') {
        out.push((start, line.trim_end_matches('\n')));
        start += line.len();
    }
    out
}

/// A compact, deterministic summary of a TS/JS module's PUBLIC surface — the names it
/// `export`s and the shape of any `*Props` type it declares — for injection into a sibling
/// file's authoring prompt.
///
/// The hosted scaffolder authors one file per model step and can carry only a bounded number
/// of full sibling BODIES forward (the `n_tokens_all <= n_batch` decode guard). The path list
/// alone tells the model a sibling EXISTS but not what it exports or what props it declares, so
/// a later file imports a symbol a sibling never exported, or passes flat props to a component
/// that declared one object — the App mounts and then throws. A one-line-per-file API summary
/// is small enough to carry for EVERY prior sibling, which is what lets a file import and wire
/// its siblings correctly regardless of authoring order.
///
/// Heuristic and best-effort (never fails a scaffold; residue is caught by the serve-time
/// `tsc --noEmit` gate). Returns `None` for a non-source file or a body with no detectable
/// surface. Deterministic: the same body yields byte-identical output.
#[must_use]
pub fn distill_module_api(path: &str, body: &[u8]) -> Option<String> {
    if !is_ts_source(path) {
        return None;
    }
    let text = std::str::from_utf8(body).ok()?;
    let is_jsx = matches!(path.rsplit('.').next(), Some("tsx" | "jsx"));
    // Names of every export, PLUS a rich signature for each const/function export — a component's
    // prop list and a hook/util's return-object shape.
    let names = exported_symbols(text);
    let rich = describe_exports(text, is_jsx);
    // `*Props` type shapes — declared in a `.ts` types module as often as a `.tsx`, so scan both.
    let props = prop_shapes(text);
    if names.is_empty() && rich.is_empty() && props.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    if !names.is_empty() {
        parts.push(format!("exports {}", names.join(", ")));
    }
    // Rich per-export detail (drop entries that add nothing over the bare name already listed).
    for r in rich {
        if r.contains('(') || r.contains("returns") {
            parts.push(r);
        }
    }
    parts.extend(props);
    let mut summary = parts.join("; ");
    if summary.len() > MAX_SIBLING_API_BYTES {
        // Truncate on a char boundary (multi-byte-safe).
        let mut cut = MAX_SIBLING_API_BYTES;
        while !summary.is_char_boundary(cut) {
            cut -= 1;
        }
        summary.truncate(cut);
        summary.push('…');
    }
    Some(summary)
}

/// Build the authoring directive for one project file. `framework`:
/// - `None` — the agentic (scheduled) lane: the fixed-skeleton markdown/JSON files.
/// - `Some(fw)` — a hosted (Experience) framework project: the directive names the
///   framework, hands the model the COMPLETE planned file set so it IMPORTS from sibling
///   modules instead of inlining them (killing the single-file monolith), and demands
///   production-quality separation. `all_paths` is passed as prompt TEXT only (cheap;
///   orthogonal to the bounded sibling-BODY context that guards the model's decode batch).
///
/// `sibling_apis` are `(path, summary)` pairs from [`distill_module_api`] for the siblings
/// ALREADY written — the export/prop contract the path list cannot convey. They are what stop
/// a hosted file from importing a symbol a sibling never exported or passing props a component
/// never declared; injected as prompt text (hosted lane only), tiny enough to carry all of them.
///
/// GR15: the committed answer IS the file body verbatim (reasoning is stripped by the
/// recipe), so the directive asks for ONLY the body — no commentary, no fences.
#[must_use]
pub fn authoring_prompt(
    path: &str,
    role: &str,
    goal: &str,
    framework: Option<&str>,
    all_paths: &[&str],
    has_siblings: bool,
    sibling_apis: &[(String, String)],
) -> String {
    let siblings = if has_siblings {
        "The attached context shows the most recent sibling files already written; keep this file \
         consistent with them. "
    } else {
        ""
    };
    match framework {
        // Hosted (Experience) lane — a real, SEPARATED framework project.
        Some(fw) => {
            let tree = if all_paths.len() > 1 {
                format!(
                    "This file is ONE file of a project whose COMPLETE source set is: {}. \
                     Import what you need from the sibling modules in that set (components, \
                     styles, hooks, types) instead of inlining or re-declaring their content, and \
                     do NOT redefine a file that appears elsewhere in the set. ",
                    all_paths.join(", ")
                )
            } else {
                String::new()
            };
            let apis = if sibling_apis.is_empty() {
                String::new()
            } else {
                let list = sibling_apis
                    .iter()
                    .map(|(p, api)| format!("`{p}` {api}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "The sibling modules already written expose exactly these APIs — import ONLY \
                     these names, and when you render a sibling component pass EXACTLY the props it \
                     declares; do NOT invent an export or a prop that is not listed:\n{list}\n"
                )
            };
            format!(
                "You are authoring one file of {label}.\n\
                 App goal: {goal}\n\n\
                 Write the COMPLETE, production-quality contents of the file `{path}` — {role}. \
                 {tree}{apis}{siblings}\
                 Return ONLY the file body — no commentary, no explanation, and no markdown code \
                 fences.",
                label = framework_label(fw),
            )
        }
        // Scheduled (agentic) lane — the fixed agentic-app skeleton (unchanged directive).
        None => format!(
            "You are scaffolding files for a durable, governed agentic application.\n\
             App goal: {goal}\n\n\
             Write the COMPLETE contents of the file `{path}` — {role}. {siblings}\
             Return ONLY the file body — no commentary, no explanation, and no markdown code fences.",
        ),
    }
}

/// `true` iff `body` is empty or whitespace-only (the GR15 fail-closed guard — a
/// stripped reasoning block that produced no body must never advance the manifest).
#[must_use]
pub fn body_is_empty(body: &[u8]) -> bool {
    body.iter().all(u8::is_ascii_whitespace)
}

/// Strip a markdown code fence that WRAPS the whole body, returning the inner bytes.
///
/// [`authoring_prompt`] asks for "no markdown code fences" on both lanes, and that is
/// the whole enforcement there has ever been. A model that ignores it puts ```` ```tsx ````
/// on line 1 of `src/App.tsx`, which is a syntax error the bundler reports from a file the
/// user never wrote — so one disobeyed sentence bricks the project.
///
/// DELIBERATELY NARROWER than `manifest::strip_json_wrappers`, which strips a leading fence
/// and then everything past the LAST ```` ``` ````. That is right for JSON (the payload can
/// contain no fence of its own) and wrong here: a README's body legitimately CONTAINS fenced
/// blocks, and `rfind` would eat the last one's content. The rule is therefore positional —
/// strip only when the first line is a bare fence opener (```` ``` ```` plus an optional
/// language tag, nothing else) AND the last non-blank line is a bare closing fence. A body
/// whose interior holds code blocks is returned untouched, because its first line is prose.
///
/// Byte-identical on the happy path (returns a subslice of the input), so a body the model
/// got right is content-addressed to exactly the ref it already had. Total + panic-free.
#[must_use]
pub fn strip_code_fence(body: &[u8]) -> &[u8] {
    let text = body.strip_suffix(b"\n").unwrap_or(body);
    let Ok(text) = std::str::from_utf8(text) else {
        return body; // not UTF-8 ⇒ not a fence; leave it exactly as authored
    };
    let mut lines = text.lines();
    let Some(first) = lines.next() else {
        return body;
    };
    // Opener: ``` + an optional language tag, and NOTHING else on the line.
    let Some(tag) = first.trim_end().strip_prefix("```") else {
        return body;
    };
    if !tag
        .trim()
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.' | b'#'))
    {
        return body;
    }
    // Closer: the last non-blank line is a bare ``` (no trailing tag).
    let Some(close) = text.lines().rev().find(|l| !l.trim().is_empty()) else {
        return body;
    };
    if close.trim() != "```" {
        return body;
    }
    // A single line that is both opener and closer is a degenerate ```` ``` ```` — nothing
    // to unwrap, and treating it as a wrap would silently produce an empty body.
    let open_len = first.len() + 1; // + the newline the opener must be followed by
    if text.len() < open_len {
        return body;
    }
    let inner = &text[open_len..];
    let Some(end) = inner.rfind("```") else {
        return body;
    };
    inner[..end].trim_end().as_bytes()
}

/// Resolve `files_done` / `files_pending` over the PLANNED file set (the dynamic
/// manifest, or the fixed skeleton as a fallback) given the branch manifest's
/// current path set. Pure — the host calls this from `status` with the planned
/// paths it read from the committed manifest marker (or the skeleton).
#[must_use]
pub fn split_done_pending(
    planned_paths: &[String],
    manifest_paths: &std::collections::BTreeSet<String>,
) -> (Vec<String>, Vec<String>) {
    let mut done = Vec::new();
    let mut pending = Vec::new();
    for p in planned_paths {
        if manifest_paths.contains(p) {
            done.push(p.clone());
        } else {
            pending.push(p.clone());
        }
    }
    (done, pending)
}

/// Derive a phase honestly from the skeleton coverage when no tracker entry exists
/// (e.g. after a restart): Done iff complete, Planning iff empty, else Writing.
#[must_use]
pub fn derive_phase(files_done: &[String], files_pending: &[String]) -> ScaffoldPhase {
    if files_pending.is_empty() && !files_done.is_empty() {
        ScaffoldPhase::Done
    } else if files_done.is_empty() {
        ScaffoldPhase::Planning
    } else {
        ScaffoldPhase::Writing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_paths_are_stable() {
        // The deterministic e2e pins exactly these paths; guard the contract here.
        let paths: Vec<&str> = SKELETON.iter().map(|f| f.path).collect();
        assert_eq!(
            paths,
            vec![
                "README.md",
                "app.json",
                "prompts/system.md",
                "rules/guardrails.md",
                "skills/main.md",
            ]
        );
    }

    #[test]
    fn authoring_prompt_demands_body_only_and_includes_goal() {
        // Agentic (scheduled) lane: framework = None, the unchanged skeleton directive.
        let p = authoring_prompt(
            SKELETON[0].path,
            SKELETON[0].role,
            "summarize PDFs",
            None,
            &[],
            false,
            &[],
        );
        assert!(p.contains("summarize PDFs"));
        assert!(p.contains("README.md"));
        assert!(p.contains("no markdown code fences"));
        assert!(p.contains("agentic application"));
        assert!(!p.contains("sibling files")); // no siblings on the first file
        let p2 = authoring_prompt(
            SKELETON[2].path,
            SKELETON[2].role,
            "summarize PDFs",
            None,
            &[],
            true,
            &[],
        );
        assert!(p2.contains("sibling files")); // siblings included once prior files exist
    }

    #[test]
    fn authoring_prompt_hosted_names_the_framework_and_the_file_set() {
        // Hosted (Experience) lane: framework = Some, the separated-project directive that
        // hands the model the whole file set so it imports siblings (no monolith).
        let all = ["src/App.tsx", "src/App.css", "src/components/Card.tsx"];
        let p = authoring_prompt(
            "src/App.tsx",
            "the root component",
            "a recipe card",
            Some("vite_react"),
            &all,
            false,
            &[],
        );
        assert!(p.contains("React"));
        assert!(p.contains("src/components/Card.tsx")); // the full set is in the prompt
        assert!(p.contains("Import")); // import-siblings directive (kills the monolith)
        assert!(p.contains("no markdown code fences"));
    }

    #[test]
    fn authoring_prompt_hosted_injects_sibling_apis() {
        // The distilled sibling APIs ride the prompt (hosted lane) so a file imports EXACTLY
        // what a sibling exported and passes EXACTLY the props a component declared.
        let apis = vec![
            (
                "src/actions.ts".to_string(),
                "exports increment, decrement, reset".to_string(),
            ),
            (
                "src/components/ResultDisplay.tsx".to_string(),
                "exports ResultDisplay; ResultDisplayProps { state: CalculatorState }".to_string(),
            ),
        ];
        let p = authoring_prompt(
            "src/App.tsx",
            "the root component",
            "a tip calculator",
            Some("vite_react"),
            &[
                "src/App.tsx",
                "src/actions.ts",
                "src/components/ResultDisplay.tsx",
            ],
            true,
            &apis,
        );
        assert!(p.contains("expose exactly these APIs"));
        assert!(p.contains("increment, decrement, reset"));
        assert!(p.contains("ResultDisplayProps { state: CalculatorState }"));
        // The scheduled lane never gets an API block, even if (hypothetically) handed one.
        let sched = authoring_prompt("README.md", "the readme", "x", None, &[], true, &apis);
        assert!(!sched.contains("expose exactly these APIs"));
    }

    #[test]
    fn distill_module_api_reports_exports_and_prop_shapes() {
        // The two real hosted-scaffold breaks, as fixtures.
        // (1) a hook imports a symbol the actions module never exported.
        let actions = b"export const increment = (n: number) => n + 1;\n\
                        export const decrement = (n: number) => n - 1;\n\
                        export function reset() { return 0; }\n";
        let api = distill_module_api("src/actions.ts", actions).unwrap();
        assert!(api.contains("exports increment, decrement, reset"), "{api}");
        assert!(!api.contains("CounterActions")); // the symbol the hook wrongly imported

        // (2) a parent passes flat props to a component that declared one object.
        let rd = b"import { CalculatorState } from './types';\n\
                   interface ResultDisplayProps { state: CalculatorState }\n\
                   export function ResultDisplay({ state }: ResultDisplayProps) {\n\
                       return <div>{state.total}</div>;\n\
                   }\n";
        let api = distill_module_api("src/components/ResultDisplay.tsx", rd).unwrap();
        assert!(api.contains("exports ResultDisplay"), "{api}");
        assert!(
            api.contains("ResultDisplayProps { state: CalculatorState }"),
            "{api}"
        );

        // Determinism: same body ⇒ byte-identical summary.
        assert_eq!(
            distill_module_api("src/actions.ts", actions),
            Some(api_of(actions))
        );
    }

    fn api_of(body: &[u8]) -> String {
        distill_module_api("src/actions.ts", body).unwrap()
    }

    #[test]
    fn distill_module_api_handles_export_forms_and_skips_non_source() {
        assert_eq!(
            distill_module_api("README.md", b"# hi\nexport nothing"),
            None
        );
        assert_eq!(distill_module_api("app.json", b"{}"), None);
        // default + named-list + `as` alias.
        let m = b"export default function App() {}\n\
                  const a = 1; const b = 2;\n\
                  export { a, b as bee };\n\
                  export type Props = { x: number };\n";
        let api = distill_module_api("src/x.tsx", m).unwrap();
        assert!(api.contains("default"), "{api}");
        assert!(api.contains("bee"), "{api}"); // the alias, not `b`
        assert!(!api.contains(" b,") && !api.contains("exports a, b;"));
        assert!(api.contains("Props { x: number }"), "{api}");
    }

    #[test]
    fn distill_module_api_captures_hook_returns_and_component_props() {
        // The exact tip-calculator patterns the live proof exposed — the two drifts a bare
        // export-name list could not convey.

        // (1) A hook returning an inline object (with a NESTED useMemo return that must be
        //     ignored) ⇒ the OUTER return keys, so the entry destructures the right fields.
        let hook = b"import { useState, useMemo } from 'react';\n\
            export const useTipCalculator = () => {\n\
            \x20 const [inputState, setInputState] = useState({ billAmount: '0' });\n\
            \x20 const updateInput = (f, v) => setInputState(p => ({ ...p, [f]: v }));\n\
            \x20 const calculation = useMemo(() => { return { billAmount: 0, tipAmount: 0 }; }, [inputState]);\n\
            \x20 return { inputState, updateInput, calculation };\n\
            };\n";
        let api = distill_module_api("src/hooks/useTipCalculator.ts", hook).unwrap();
        assert!(
            api.contains("useTipCalculator() returns { inputState, updateInput, calculation }"),
            "{api}"
        );
        assert!(
            !api.contains("tipAmount"),
            "the nested useMemo return must be ignored: {api}"
        );

        // (2) A propless component, defined then default-exported ⇒ "no props", so the entry
        //     renders <TipInput /> instead of passing props it does not accept.
        let propless = b"import React from 'react';\n\
            import { useTipCalculator } from '../hooks/useTipCalculator';\n\
            const TipInput: React.FC = () => {\n  const { inputState } = useTipCalculator();\n  return <div>{inputState.billAmount}</div>;\n};\n\
            export default TipInput;\n";
        let api = distill_module_api("src/components/TipInput.tsx", propless).unwrap();
        assert!(api.contains("default TipInput(no props)"), "{api}");

        // (3) A typed component (React.FC<Props>, default-exported) ⇒ its props TYPE, so the
        //     entry passes exactly those props.
        let typed = b"import React from 'react';\n\
            import { TipResultProps } from '../types';\n\
            const TipResult: React.FC<TipResultProps> = ({ tipAmount, totalAmount }) => {\n  return <p>{tipAmount}{totalAmount}</p>;\n};\n\
            export default TipResult;\n";
        let api = distill_module_api("src/components/TipResult.tsx", typed).unwrap();
        assert!(
            api.contains("default TipResult(props: TipResultProps)"),
            "{api}"
        );

        // (4) A `.ts` types module's `*Props` shape is captured (previously .tsx-only).
        let type_module = b"export type TipCalculation = { tipAmount: number };\n\
            export interface TipResultProps { tipAmount: number; totalAmount: number }\n";
        let api = distill_module_api("src/types.ts", type_module).unwrap();
        assert!(
            api.contains("TipResultProps { tipAmount: number; totalAmount: number }"),
            "{api}"
        );
    }

    #[test]
    fn body_is_empty_detects_whitespace_only() {
        assert!(body_is_empty(b""));
        assert!(body_is_empty(b"   \n\t "));
        assert!(!body_is_empty(b"x"));
        assert!(!body_is_empty(b"  hi  "));
    }

    #[test]
    fn strip_code_fence_unwraps_a_fenced_source_file() {
        // The failure this exists for: ```tsx on line 1 of src/App.tsx is a syntax error
        // in a file the user never wrote.
        let body = b"```tsx\nexport default function App() {\n  return <p>hi</p>;\n}\n```\n";
        assert_eq!(
            strip_code_fence(body),
            b"export default function App() {\n  return <p>hi</p>;\n}".as_slice()
        );
        // A bare fence (no language tag) unwraps too.
        assert_eq!(strip_code_fence(b"```\nplain\n```"), b"plain".as_slice());
    }

    #[test]
    fn strip_code_fence_leaves_a_markdown_body_with_inner_fences_alone() {
        // The regression `strip_json_wrappers`' rfind rule would cause: this body's LAST
        // ``` closes a legitimate block, and eating up to it would delete `npm run dev`.
        let body = b"# Tip Calculator\n\nRun it:\n\n```sh\nnpm run dev\n```\n";
        assert_eq!(strip_code_fence(body), body.as_slice());
        // Prose after the closing fence is likewise untouched (first line is not a fence).
        let trailing = b"Intro.\n\n```js\nx\n```\n\nOutro.\n";
        assert_eq!(strip_code_fence(trailing), trailing.as_slice());
    }

    #[test]
    fn strip_code_fence_is_conservative_about_what_counts_as_a_wrap() {
        // An opener carrying prose is not a fence line — leave it.
        let prose = b"```js let x = 1\ncode\n```";
        assert_eq!(strip_code_fence(prose), prose.as_slice());
        // No closing fence ⇒ not a wrap.
        let unclosed = b"```tsx\ncode\n";
        assert_eq!(strip_code_fence(unclosed), unclosed.as_slice());
        // A closer carrying a tag is not a bare closer.
        let tagged_close = b"```tsx\ncode\n```tsx";
        assert_eq!(strip_code_fence(tagged_close), tagged_close.as_slice());
        // A lone fence has nothing to unwrap.
        assert_eq!(strip_code_fence(b"```"), b"```".as_slice());
    }

    #[test]
    fn strip_code_fence_leaves_an_empty_wrap_for_the_fail_closed_guard() {
        // An empty fenced block must NOT sneak past as a "successful" write — it strips to
        // nothing, and `body_is_empty` is what refuses to advance the branch.
        let stripped = strip_code_fence(b"```tsx\n```");
        assert!(body_is_empty(stripped));
    }

    #[test]
    fn strip_code_fence_is_byte_identical_on_the_happy_path() {
        // A body the model got right must content-address to exactly the ref it already
        // had — the whole point of returning a subslice rather than rebuilding.
        let clean = b"import React from 'react';\n\nexport const x = 1;\n";
        assert_eq!(strip_code_fence(clean), clean.as_slice());
        // Non-UTF-8 is passed through verbatim rather than guessed at.
        let binary: &[u8] = &[0xff, 0xfe, 0x00, 0x01];
        assert_eq!(strip_code_fence(binary), binary);
    }

    #[test]
    fn split_and_derive_phase_cover_the_states() {
        let planned: Vec<String> = SKELETON.iter().map(|f| f.path.to_string()).collect();

        let none = std::collections::BTreeSet::<String>::new();
        let (d, p) = split_done_pending(&planned, &none);
        assert!(d.is_empty());
        assert_eq!(p.len(), SKELETON.len());
        assert_eq!(derive_phase(&d, &p), ScaffoldPhase::Planning);

        let all: std::collections::BTreeSet<String> =
            SKELETON.iter().map(|f| f.path.to_string()).collect();
        let (d, p) = split_done_pending(&planned, &all);
        assert_eq!(d.len(), SKELETON.len());
        assert!(p.is_empty());
        assert_eq!(derive_phase(&d, &p), ScaffoldPhase::Done);

        let some: std::collections::BTreeSet<String> = [SKELETON[0].path.to_string()].into();
        let (d, p) = split_done_pending(&planned, &some);
        assert_eq!(derive_phase(&d, &p), ScaffoldPhase::Writing);
    }

    #[test]
    fn split_done_pending_follows_a_dynamic_manifest() {
        // The planned set is model-chosen, not the fixed skeleton.
        let planned = vec![
            "package.json".to_string(),
            "src/App.tsx".to_string(),
            "src/App.css".to_string(),
        ];
        let present: std::collections::BTreeSet<String> =
            ["package.json".to_string(), "src/App.tsx".to_string()].into();
        let (d, p) = split_done_pending(&planned, &present);
        assert_eq!(
            d,
            vec!["package.json".to_string(), "src/App.tsx".to_string()]
        );
        assert_eq!(p, vec!["src/App.css".to_string()]);
        assert_eq!(derive_phase(&d, &p), ScaffoldPhase::Writing);
    }
}
