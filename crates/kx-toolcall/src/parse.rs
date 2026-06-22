//! The fail-closed parse: [`parse_tool_call`] + the args-size cap
//! [`max_args_bytes`]. Moved verbatim from `kx-model-harness::toolcall`
//! (PR-2d-1); the 13 gate tests moved with it and pin the behavior.

use kx_mote::{ToolName, ToolVersion};
use kx_warrant::{ToolGrant, WarrantSpec};
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::types::{DecodeError, ToolCall};

/// The JSON envelope a model uses to propose a tool call:
/// `{"tool_call": {"name": "...", "version": "...", "args": { ... }}}`.
#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    tool_call: Option<RawToolCall>,
}

#[derive(Deserialize)]
struct RawToolCall {
    name: String,
    version: String,
    args: Box<RawValue>,
}

/// The per-call args-size cap (IMP-16), derived from the warrant's output ceiling
/// (`max_output_tokens · 4` — the model produced the args, so the output budget
/// bounds them). Saturating, mirroring `context::window_bytes_from_warrant`.
#[must_use]
pub fn max_args_bytes(warrant: &WarrantSpec) -> usize {
    (warrant.model_route.max_output_tokens as usize).saturating_mul(4)
}

/// Extract the JSON envelope a model wrapped in reasoning and/or a markdown code
/// fence, so the strict parser sees the bare `{ … }`. Removes a SINGLE leading
/// reasoning block — Qwen3 `<think>…</think>` OR Gemma-4 `<|channel>…<channel|>`
/// — then a surrounding markdown code fence (```` ```json … ``` ````; Gemma-4
/// reliably fences structured output).
///
/// Leading-block + structural-wrapper ONLY — we NEVER scan for `{` mid-string
/// (the fence is a defined ```` ``` ```` delimiter, not a `{` search), so the
/// strict `starts_with('{')` gate below stays the injection boundary (SN-8).
/// Mirrors `kx_planner::decode`'s extractor — the two trust seams keep the SAME
/// discipline. Total + panic-free; an unclosed reasoning tag yields `""`, which
/// the caller treats as a normal (non-call) completion (fail-closed).
fn extract_json_envelope(text: &str) -> &str {
    strip_code_fence(strip_reasoning_preamble(text))
}

/// Strip a SINGLE leading reasoning block: Qwen3 `<think>…</think>` or Gemma-4
/// `<|channel>…<channel|>`. An unclosed tag yields `""`.
fn strip_reasoning_preamble(text: &str) -> &str {
    let t = text.trim_start();
    for (open, close) in [("<think>", "</think>"), ("<|channel>", "<channel|>")] {
        if let Some(rest) = t.strip_prefix(open) {
            return match rest.find(close) {
                Some(i) => rest[i + close.len()..].trim_start(),
                None => "",
            };
        }
    }
    t
}

/// Strip a surrounding markdown code fence (```` ``` ````), optionally tagged
/// (```` ```json ````). No fence ⇒ `text` trimmed. Total + panic-free.
fn strip_code_fence(text: &str) -> &str {
    let t = text.trim();
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

/// Gemma-4's NATIVE tool-call open delimiter (`<|tool_call>call:NAME{ARGS}<tool_call|>`).
const GEMMA_TOOL_OPEN: &str = "<|tool_call>";
/// The optional `call:` marker after the open delimiter (observed: `call:fs_list{}`).
const GEMMA_CALL_MARKER: &str = "call:";

/// A model-NATIVE (non-envelope) call shape, post-extraction: the raw tool name
/// and the verbatim args-object bytes. The version is resolved against the grant
/// set by the caller (Gemma emits no version).
struct NativeCall<'a> {
    raw_name: &'a str,
    args: &'a str,
}

/// Extract a Gemma-4 native `<|tool_call>call:NAME{ARGS}<tool_call|>` call from the
/// (reasoning-stripped) text, or `None` if the text is not this shape. NAME is the
/// run after the (optional) `call:` marker up to the FIRST `{` — a DEFINED
/// NAME/ARGS boundary, exactly like the markdown fence (NEVER a mid-string `{`
/// search, so the SN-8 injection boundary is unchanged: only bytes the model
/// fenced inside `<|tool_call>…` are promoted to a call). The `<tool_call|>` close
/// is optional (truncation-tolerant) — `balanced_object` bounds the args object so
/// trailing prose / the close delim can never leak in. Total + panic-free.
fn extract_gemma_native(text: &str) -> Option<NativeCall<'_>> {
    let after_open = text.trim_start().strip_prefix(GEMMA_TOOL_OPEN)?;
    let after_marker = after_open
        .trim_start()
        .strip_prefix(GEMMA_CALL_MARKER)
        .unwrap_or_else(|| after_open.trim_start());
    let brace = after_marker.find('{')?;
    let raw_name = after_marker[..brace].trim();
    if raw_name.is_empty() {
        return None;
    }
    let args = balanced_object(&after_marker[brace..])?;
    Some(NativeCall { raw_name, args })
}

/// Return the prefix of `s` (which MUST start with `{`) spanning the first
/// brace-balanced JSON object, ignoring braces inside double-quoted strings
/// (with `\"` escapes). `None` if unbalanced or past `MAX_DEPTH`. Total +
/// panic-free; the depth bound rejects pathological nesting cheaply (serde
/// re-parses for shape downstream).
fn balanced_object(s: &str) -> Option<&str> {
    const MAX_DEPTH: usize = 64;
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return None;
                }
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Llama-3.1/3.2's native tool-call open delimiter (`<|python_tag|>{"name":…}`).
const PYTHON_TAG_OPEN: &str = "<|python_tag|>";
/// Qwen3/Hermes XML-ish tool-call open tag (`<tool_call>{"name":…}</tool_call>`).
/// DISTINCT from Gemma's `<|tool_call>` (note the `|`): `strip_prefix` is exact, so
/// the two delimiters never collide, and the Gemma arm runs first.
const XML_TOOL_OPEN: &str = "<tool_call>";

/// Strip a DEFINED open delimiter, then return the brace-balanced inner `{ … }`
/// object that follows it (after optional whitespace) — or `None`. Shared by the
/// `<|python_tag|>` and `<tool_call>` shapes, which both wrap a
/// `{"name":…, "arguments"|"parameters"|"args":…}` object after a marker. NEVER a
/// mid-string `{` search (the marker is the boundary, and `balanced_object` bounds
/// the object so a `</tool_call>` close tag / trailing prose can never leak in) —
/// so the SN-8 injection boundary is unchanged. Total + panic-free.
fn marked_object<'a>(text: &'a str, open: &str) -> Option<&'a str> {
    let after = text.trim_start().strip_prefix(open)?;
    balanced_object(after.trim_start())
}

/// Decode an inner `{"name":…, <args-alias>:…}` object (the body of a
/// `<|python_tag|>` / `<tool_call>` shape) into `(raw_name, args_bytes)`, or `None`
/// if it is not a recognizable named-tool object (fail-closed → the caller falls
/// through to a normal completion). The args bag is accepted under ANY of
/// `args` | `arguments` | `parameters` (models differ) — EXACTLY one present (two or
/// more ⇒ `None`, ambiguous), as either a JSON object (carried verbatim) OR a
/// pre-serialized JSON STRING (unescaped to its inner JSON — some models emit
/// `"arguments":"{…}"`). Requires a non-empty `name`.
///
/// SN-8: this widens only ENVELOPE recognition — the `name` and the args bytes are
/// preserved and still flow through `resolve_granted_name` (exact grant membership)
/// and the downstream schema `validate_args`. Unknown sibling keys are ignored here
/// (tolerant envelope), but a smuggled ARG key is still rejected by the typed schema
/// downstream, so the authority surface is unchanged. Total + panic-free.
fn decode_named_object(obj: &str, require_explicit_args: bool) -> Option<(String, Vec<u8>)> {
    #[derive(Deserialize)]
    struct Named<'a> {
        name: Option<String>,
        #[serde(borrow, default)]
        args: Option<&'a RawValue>,
        #[serde(borrow, default)]
        arguments: Option<&'a RawValue>,
        #[serde(borrow, default)]
        parameters: Option<&'a RawValue>,
    }
    let parsed: Named = serde_json::from_str(obj).ok()?;
    let name = parsed.name?;
    if name.trim().is_empty() {
        return None;
    }
    let mut present = [parsed.args, parsed.arguments, parsed.parameters]
        .into_iter()
        .flatten();
    let args_bytes = match present.next() {
        // No args alias. A MARKED caller treats absent as an empty args object
        // (matches `validate_args`' empty == `{}`); a MARKERLESS caller (PR-R1)
        // REQUIRES an explicit args bag (commitment-aware — a bare object with only
        // a `name` and no args key is far likelier prose than a tool call).
        None if require_explicit_args => return None,
        None => b"{}".to_vec(),
        Some(v) => {
            if present.next().is_some() {
                return None; // two+ aliases ⇒ ambiguous ⇒ fail-closed
            }
            args_value_bytes(v)?
        }
    };
    Some((name, args_bytes))
}

/// Resolve a tool-call args VALUE (a `RawValue`) to verbatim args-object bytes: a
/// JSON object is carried byte-for-byte; a pre-serialized JSON STRING is unescaped
/// to its inner JSON (then JSON5-repaired + schema-validated downstream); any other
/// kind (array/scalar) ⇒ `None` (a tool's args are an object). Total + panic-free.
fn args_value_bytes(v: &RawValue) -> Option<Vec<u8>> {
    let raw = v.get();
    let head = raw.trim_start();
    if head.starts_with('{') {
        Some(raw.as_bytes().to_vec())
    } else if head.starts_with('"') {
        // A pre-serialized JSON string: unescape to the inner JSON bytes.
        serde_json::from_str::<String>(raw)
            .ok()
            .map(String::into_bytes)
    } else {
        None
    }
}

/// Separator-canonicalize a single name segment: `_`→`-` (matching how Gemma
/// renders `fs-list` as `fs_list`), trimmed. This is the EXISTING gate
/// normalization, factored out — NEVER a fuzzy/similarity/edit-distance remap
/// (SN-8: no similarity on any identity path).
fn canon(s: &str) -> String {
    s.trim().replace('_', "-")
}

/// Reduce a model-emitted name to its identity core: drop an `@version` tail, then
/// a `:remote` tail — decorations a grant's `tool_id` never carries (the model
/// reconstructs `<id>:<remote>` from the menu, or copies the `tool.<id>@<ver>`
/// label), then `canon`. The version is authoritatively the grant's (taken by the
/// caller) and the remote-name is the tool's internal wiring, never an identity the
/// warrant grants on — so dropping them cannot reach a tool outside the grant set.
/// Total + panic-free (`split` always yields at least one element).
fn model_name_core(raw_name: &str) -> String {
    let no_ver = raw_name.split('@').next().unwrap_or(raw_name);
    let no_remote = no_ver.split(':').next().unwrap_or(no_ver);
    canon(no_remote)
}

/// True iff `target` (an already-`canon`'d model name core) addresses `tool_id` by
/// one of its canonical aliases: the FULL id, OR ANY `/`-delimited segment of it. A
/// dialed/local MCP tool is registered `<server>/<remote>`, and real models propose
/// EITHER end — the short leaf `<remote>` (e.g. `echo`) OR the server prefix
/// `<server>` (Gemma-4 emits the bare `mcp-echo` for `mcp-echo/echo`). EXACT segment
/// equality ONLY — never a prefix/substring/fuzzy match (SN-8); cross-grant ambiguity
/// (two grants sharing the addressed segment) is fail-closed in [`resolve_granted_name`].
fn id_matches(target: &str, tool_id: &str) -> bool {
    let full = canon(tool_id);
    if full == target {
        return true;
    }
    full.split('/').any(|seg| !seg.is_empty() && seg == target)
}

/// Resolve a model-emitted (often separator-variant, version-less, or
/// namespace-stripped) tool name to a GRANTED `(ToolName, ToolVersion)`, SN-8-safe.
/// Resolution = the UNIQUE granted tool addressed by the model's name core (its
/// full id OR the leaf after the last `/`, both `canon`-normalized). ANY ambiguity
/// (two distinct grants addressed by the same core) ⇒ `None` (fail-closed — no
/// guessing). The returned version is whatever the grant pins, so the downstream
/// `tool_grants` membership is exact by construction. NEVER widens the grant set:
/// the result is always an element of `warrant.tool_grants` (cloned) or `None`.
fn resolve_granted_name(raw_name: &str, warrant: &WarrantSpec) -> Option<ToolGrant> {
    let target = model_name_core(raw_name);
    if target.is_empty() {
        return None; // a name that canonicalizes to nothing addresses no grant
    }
    let mut hit: Option<&ToolGrant> = None;
    for g in &warrant.tool_grants {
        if id_matches(&target, &g.tool_id.0) {
            if hit.is_some() {
                return None; // ambiguous ⇒ fail-closed (SN-8)
            }
            hit = Some(g);
        }
    }
    hit.cloned()
}

/// Resolve a MARKERLESS named call to a granted `ToolCall`, fail-closed. Unlike the
/// MARKED arms (a marker IS the model's commitment, so a bad name is a loud refusal),
/// a markerless object carries no commitment signal — so a name that addresses NO
/// grant is a normal completion (`Ok(None)`), NEVER a false-positive refusal. The
/// authority surface is unchanged: `resolve_granted_name` (exact grant membership,
/// SN-8) + the downstream schema; only ENVELOPE recognition widens.
fn markerless_call(
    raw_name: &str,
    args_bytes: Vec<u8>,
    warrant: &WarrantSpec,
    max_args_bytes: usize,
) -> Result<Option<ToolCall>, DecodeError> {
    let Some(grant) = resolve_granted_name(raw_name, warrant) else {
        return Ok(None); // markerless: a non-granted name is prose, not a refusal
    };
    if args_bytes.len() > max_args_bytes {
        return Err(DecodeError::Oversize {
            got: args_bytes.len(),
            max: max_args_bytes,
        });
    }
    Ok(Some(ToolCall {
        name: grant.tool_id,
        version: grant.tool_version,
        args_bytes,
    }))
}

/// PR-R1: the COMMITMENT-AWARE markerless tool-call shapes — the JSON-envelope arm's
/// complement to the marked detectors. Recognizes two shapes more model families emit
/// with no `tool_call` wrapper and no marker: a bare named object
/// `{"name":…, "arguments":{…}}` (`OpenAI` / Hermes), and a SINGLE-element
/// `{"tool_calls":[ {"name":…, "arguments":{…}} ]}` wrapper. Each fires ONLY when the
/// name resolves to a granted tool AND an EXPLICIT args bag is present (the
/// commitment-aware guard — see [`markerless_call`] / [`decode_named_object`]);
/// otherwise it degrades to a normal completion (never a false-positive refusal). A
/// MULTI-element `tool_calls` array is DEFERRED — multiple-tool-calls-per-turn is a
/// coordinator loop-semantics change (the react loop freezes one `Tool` fact/turn) —
/// and yields `None` with NO silent first-element cap. Total + panic-free.
fn decode_markerless(
    trimmed: &str,
    warrant: &WarrantSpec,
    max_args_bytes: usize,
) -> Result<Option<ToolCall>, DecodeError> {
    // The `{"tool_calls":[…]}` wrapper shape (declared here, before the first stmt).
    #[derive(Deserialize)]
    struct ToolCalls<'a> {
        #[serde(borrow)]
        tool_calls: Vec<&'a RawValue>,
    }
    // (a) a bare named object — top-level `{"name":…, <args alias>:{…}}`.
    if let Some((raw_name, args_bytes)) = decode_named_object(trimmed, true) {
        return markerless_call(&raw_name, args_bytes, warrant, max_args_bytes);
    }
    // (b) a `{"tool_calls":[…]}` wrapper (OpenAI plural form). ONLY a single call is
    //     accepted; a 0- or multi-element array falls through (deferred, no silent cap).
    if let Ok(wrapper) = serde_json::from_str::<ToolCalls>(trimmed) {
        if wrapper.tool_calls.len() == 1 {
            if let Some((raw_name, args_bytes)) =
                decode_named_object(wrapper.tool_calls[0].get(), true)
            {
                return markerless_call(&raw_name, args_bytes, warrant, max_args_bytes);
            }
        }
    }
    Ok(None)
}

/// Decode a model-proposed tool call from raw model output, fail-closed.
///
/// Returns `Ok(None)` for a normal completion (prose, non-envelope JSON, or — the
/// important security default — *any* output when the warrant grants no tools).
/// Returns `Ok(Some(call))` for a well-formed, warrant-granted, size-bounded call.
/// Returns `Err` when the model committed to a tool-call envelope that is malformed,
/// names an ungranted tool, or overshoots the args cap.
///
/// A leading `<think>…</think>` block (Qwen3 reasoning) is stripped before the
/// strict parse; everything after it is still gated by `starts_with('{')`.
///
/// Total + panic-free over arbitrary `bytes`.
///
/// # Errors
///
/// [`DecodeError::Malformed`] when the output committed to a JSON object but the
/// envelope is malformed/truncated/trailing-garbage; [`DecodeError::UngrantedTool`]
/// when the proposal names a tool outside `warrant.tool_grants` (SN-8);
/// [`DecodeError::Oversize`] when the args exceed `max_args_bytes` (IMP-16).
pub fn parse_tool_call(
    bytes: &[u8],
    warrant: &WarrantSpec,
    max_args_bytes: usize,
) -> Result<Option<ToolCall>, DecodeError> {
    // (0) No grants ⇒ no tool can ever be called. Preserves the M5.1 leaf path
    //     byte-for-byte (every existing harness row grants no tools) AND is the
    //     security default: a model cannot conjure a tool the warrant withheld.
    if warrant.tool_grants.is_empty() {
        return Ok(None);
    }

    // (1) Non-UTF-8 or not-a-JSON-object output is a normal completion, not a call.
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Ok(None);
    };
    // Strip a leading reasoning block (Qwen3 `<think>` / Gemma-4 `<|channel>`) and
    // a surrounding ```` ```json ```` fence, then require the remainder to BEGIN
    // with `{` — leading-block + structural-fence only; no mid-string scan (SN-8).
    let trimmed = extract_json_envelope(text);

    // (1a) Gemma-4 NATIVE shape: `<|tool_call>call:NAME{ARGS}<tool_call|>`. A SECOND
    //      DEFINED delimiter set (not a `{` search) — recognized BEFORE the JSON
    //      gate. Version-less + separator-variant names (`fs_list`) are resolved
    //      against the grant set, and the result is gated by the SAME exact
    //      `tool_grants` equality (SN-8). Anything not opening with this exact
    //      delimiter falls through to the JSON envelope path, byte-identical for
    //      every existing row (no current input begins with `<|tool_call>`).
    if let Some(native) = extract_gemma_native(trimmed) {
        let Some(grant) = resolve_granted_name(native.raw_name, warrant) else {
            // The model COMMITTED to a native call but named an unknown/ambiguous
            // tool ⇒ fail-closed (a bad name is a refusal, never silent prose).
            return Err(DecodeError::UngrantedTool {
                name: ToolName(native.raw_name.to_string()),
                version: ToolVersion(String::new()),
            });
        };
        let args_bytes = native.args.as_bytes().to_vec();
        if args_bytes.len() > max_args_bytes {
            return Err(DecodeError::Oversize {
                got: args_bytes.len(),
                max: max_args_bytes,
            });
        }
        return Ok(Some(ToolCall {
            name: grant.tool_id,
            version: grant.tool_version,
            args_bytes,
        }));
    }

    // (1b) Llama-3.1 `<|python_tag|>{…}` and Qwen3/Hermes `<tool_call>{…}</tool_call>`
    //      — two MORE DEFINED-delimiter shapes (markers required; never a `{` search),
    //      each wrapping a `{"name":…, "arguments"|"parameters"|"args":…}` object.
    //      The name + args flow through the SAME grant resolution + exact
    //      `tool_grants` equality (SN-8) as every other arm; the args bag tolerates
    //      the model's alias + a pre-serialized-string value. A marker that does not
    //      wrap a NAMED object falls through (like a bare Gemma marker), byte-identical
    //      for every existing row (no current input begins with these markers).
    for open in [PYTHON_TAG_OPEN, XML_TOOL_OPEN] {
        let Some(obj) = marked_object(trimmed, open) else {
            continue;
        };
        let Some((raw_name, args_bytes)) = decode_named_object(obj, false) else {
            continue; // marked but not a recognizable named call ⇒ normal completion
        };
        // The model COMMITTED to a named marked call ⇒ resolve or fail-closed (a bad
        // name is a refusal, never silent prose — mirrors the Gemma-native arm).
        let Some(grant) = resolve_granted_name(&raw_name, warrant) else {
            return Err(DecodeError::UngrantedTool {
                name: ToolName(raw_name),
                version: ToolVersion(String::new()),
            });
        };
        if args_bytes.len() > max_args_bytes {
            return Err(DecodeError::Oversize {
                got: args_bytes.len(),
                max: max_args_bytes,
            });
        }
        return Ok(Some(ToolCall {
            name: grant.tool_id,
            version: grant.tool_version,
            args_bytes,
        }));
    }

    if !trimmed.starts_with('{') {
        return Ok(None);
    }

    // (2) It looks like JSON. Parse strictly — trailing garbage / truncation /
    //     bad shape is fail-closed (the injection vector lives here).
    let envelope: Envelope = serde_json::from_str(trimmed).map_err(|e| DecodeError::Malformed {
        diagnostic: e.to_string(),
    })?;
    let Some(raw) = envelope.tool_call else {
        // No `tool_call` envelope. PR-R1: try the COMMITMENT-AWARE markerless shapes
        // (a bare `{"name":…,"arguments":…}` object, a single-element `{"tool_calls":
        // […]}` wrapper) — they fire only when the name resolves to a grant AND carry
        // an explicit args bag, else degrade to a normal completion (no false-positive
        // refusal). Otherwise: valid JSON, not a tool call ⇒ a normal completion.
        return decode_markerless(trimmed, warrant, max_args_bytes);
    };

    // (3) The model committed to a tool call. Enforce tool ∈ warrant.tool_grants.
    //     EXACT (name, version) crypto-equality is tried FIRST — byte-identical to
    //     every prior row (SN-8 / D70). Only on an exact MISS *with an empty
    //     version* (the `mcp-echo:echo` separator/version-drift shape a model emits
    //     when it copies the menu label) do we resolve the name to a UNIQUE grant
    //     and take the GRANT's version — never the model's. A NON-empty wrong
    //     version stays `UngrantedTool` (the model pinned a different tool — SN-8;
    //     keeps `ungranted_tool_is_refused` valid). The returned call is an element
    //     of `tool_grants` by construction (never widens the set).
    let name = ToolName(raw.name);
    let version = ToolVersion(raw.version);
    let exact = ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    };
    let grant = if warrant.tool_grants.contains(&exact) {
        exact
    } else if version.0.trim().is_empty() {
        match resolve_granted_name(&name.0, warrant) {
            Some(g) => g,
            None => return Err(DecodeError::UngrantedTool { name, version }),
        }
    } else {
        return Err(DecodeError::UngrantedTool { name, version });
    };

    // (4) Carry the args verbatim, size-capped (IMP-16). An args OBJECT is carried
    //     byte-for-byte (the PR-2d-1 pin); a pre-serialized JSON-STRING value (some
    //     models emit `"args":"{…}"`) is unescaped to its inner JSON, then repaired +
    //     schema-validated downstream — the envelope-side complement to PR-3's args
    //     repair. A non-object/non-string value carries verbatim (refused downstream).
    let args_bytes =
        args_value_bytes(&raw.args).unwrap_or_else(|| raw.args.get().as_bytes().to_vec());
    if args_bytes.len() > max_args_bytes {
        return Err(DecodeError::Oversize {
            got: args_bytes.len(),
            max: max_args_bytes,
        });
    }

    Ok(Some(ToolCall {
        name: grant.tool_id,
        version: grant.tool_version,
        args_bytes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_warrant::{
        ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
    };
    use std::collections::BTreeSet;

    fn warrant_granting(tool: Option<(&str, &str)>) -> WarrantSpec {
        let mut tool_grants = BTreeSet::new();
        if let Some((id, ver)) = tool {
            tool_grants.insert(ToolGrant {
                tool_id: ToolName(id.into()),
                tool_version: ToolVersion(ver.into()),
            });
        }
        WarrantSpec {
            mote_class: MoteClass::WorldMutating,
            nd_class: MoteClass::WorldMutating,
            fs_scope: FsScope::empty(),
            net_scope: NetScope::None,
            syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
            tool_grants,
            model_route: ModelRoute {
                model_id: kx_mote::ModelId("m".into()),
                max_input_tokens: 1024,
                max_output_tokens: 256,
                max_calls: 8,
            },
            resource_ceiling: ResourceCeiling {
                cpu_milli: 0,
                mem_bytes: 0,
                wall_clock_ms: 1000,
                fd_count: 0,
                disk_bytes: 0,
            },
            environment_ref: None,
            executor_class: ExecutorClass::Bwrap,
            ..Default::default()
        }
    }

    #[test]
    fn empty_grants_is_always_none() {
        let w = warrant_granting(None);
        // Even a perfectly-formed envelope yields None when nothing is granted.
        let env = br#"{"tool_call":{"name":"mcp-echo","version":"1","args":{}}}"#;
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn prose_is_a_normal_completion() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        assert_eq!(
            parse_tool_call(b"The sky is blue.", &w, 4096),
            Ok(None),
            "prose ⇒ no tool call"
        );
    }

    #[test]
    fn non_envelope_json_is_a_normal_completion() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        assert_eq!(parse_tool_call(br#"{"answer":"blue"}"#, &w, 4096), Ok(None));
    }

    #[test]
    fn well_formed_granted_call_is_decoded() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"tool_call":{"name":"mcp-echo","version":"1","args":{"q":"x"}}}"#;
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("a call");
        assert_eq!(call.name, ToolName("mcp-echo".into()));
        assert_eq!(call.version, ToolVersion("1".into()));
        assert_eq!(call.args_bytes, br#"{"q":"x"}"#.to_vec());
    }

    #[test]
    fn think_preamble_then_envelope_is_decoded() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Qwen3 thinking-mode: a reasoning block precedes the tool-call JSON.
        let env = b"<think>I should echo the query.</think>\n{\"tool_call\":{\"name\":\"mcp-echo\",\"version\":\"1\",\"args\":{\"q\":\"x\"}}}";
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("a call");
        assert_eq!(call.name, ToolName("mcp-echo".into()));
        assert_eq!(call.args_bytes, br#"{"q":"x"}"#.to_vec());
    }

    #[test]
    fn fenced_envelope_is_decoded() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Gemma-4 reliably fences structured output in a ```json block.
        let env =
            b"```json\n{\"tool_call\":{\"name\":\"mcp-echo\",\"version\":\"1\",\"args\":{\"q\":\"x\"}}}\n```";
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a fenced call");
        assert_eq!(call.name, ToolName("mcp-echo".into()));
        assert_eq!(call.args_bytes, br#"{"q":"x"}"#.to_vec());
    }

    #[test]
    fn gemma_channel_then_fenced_envelope_is_decoded() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Gemma-4: a `<|channel>thought…<channel|>` reasoning segment then a
        // fenced tool-call.
        let env = b"<|channel>thought\nI should echo.<channel|>```json\n{\"tool_call\":{\"name\":\"mcp-echo\",\"version\":\"1\",\"args\":{}}}\n```";
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a channel + fence call");
        assert_eq!(call.name, ToolName("mcp-echo".into()));
    }

    #[test]
    fn bundled_server_slash_remote_resolves_the_bare_remote_leaf() {
        // BUG-33 (PR-2 deep-test campaign finding A1): the bundled echo is now granted
        // as `mcp-echo/echo` (the <server>/<remote> convention every MCP tool uses — a
        // dialed/local tool registers `<server>/<remote>`). A capable model
        // (Gemma-4-12B) prompted to "use the echo tool" naturally proposes the bare
        // remote leaf `echo`; it MUST resolve to the grant via the leaf rule. Before
        // the fix the bundled tool was a flat `mcp-echo` (no `/`), so the bare `echo`
        // was refused `UngrantedTool` and the live ReAct chain dead-lettered with no
        // answer. SN-8: the leaf is EXACT segment equality, never prefix/substring.
        let w = warrant_granting(Some(("mcp-echo/echo", "1")));

        // (a) the bare remote leaf, version-less (JSON envelope) ⇒ resolves to the grant.
        let env = br#"{"tool_call":{"name":"echo","version":"","args":{"q":"x"}}}"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("the bare remote leaf resolves to the <server>/<remote> grant");
        assert_eq!(call.name, ToolName("mcp-echo/echo".into()));
        assert_eq!(call.version, ToolVersion("1".into())); // the GRANT's version, not the model's

        // (b) the Gemma-4 NATIVE shape with the bare leaf ⇒ resolves too.
        let native = b"<|tool_call>call:echo{\"q\":\"x\"}<tool_call|>";
        let nc = parse_tool_call(native, &w, 4096)
            .unwrap()
            .expect("native bare leaf resolves");
        assert_eq!(nc.name, ToolName("mcp-echo/echo".into()));

        // (c) the full id still resolves (exact match path).
        let env_full = br#"{"tool_call":{"name":"mcp-echo/echo","version":"1","args":{"q":"x"}}}"#;
        assert!(parse_tool_call(env_full, &w, 4096).unwrap().is_some());

        // (d) PR-R1 (live Gemma-4 finding): the SERVER PREFIX `mcp-echo` (the first
        //     `/`-segment of `mcp-echo/echo`) ALSO resolves — Gemma-4-12B emits the bare
        //     `mcp-echo` for the bundled tool. UNAMBIGUOUS here (one grant on that
        //     server), so it resolves to the grant's full id + version. EXACT segment
        //     equality (never prefix/substring): a non-segment like `mcp` stays refused.
        let env_prefix = br#"{"tool_call":{"name":"mcp-echo","version":"","args":{"q":"x"}}}"#;
        let pc = parse_tool_call(env_prefix, &w, 4096)
            .unwrap()
            .expect("the server-prefix segment resolves the unique grant on that server");
        assert_eq!(pc.name, ToolName("mcp-echo/echo".into()));
        let env_partial = br#"{"tool_call":{"name":"mcp","version":"","args":{"q":"x"}}}"#;
        assert!(
            matches!(
                parse_tool_call(env_partial, &w, 4096),
                Err(DecodeError::UngrantedTool { .. })
            ),
            "a non-segment substring (`mcp`) never resolves — exact segment equality only"
        );
    }

    #[test]
    fn shared_server_segment_is_ambiguous_fail_closed_but_distinct_leaves_resolve() {
        // SN-8: when two grants SHARE the addressed segment (the `mcp-echo` server of
        // both `mcp-echo/echo` and `mcp-echo/reverse`), the bare `mcp-echo` is
        // AMBIGUOUS ⇒ fail-closed (no guessing). The DISTINCT leaves still resolve.
        let w = warrant_granting_many(&[("mcp-echo/echo", "1"), ("mcp-echo/reverse", "2")]);
        let ambiguous = br#"{"tool_call":{"name":"mcp-echo","version":"","args":{"q":"x"}}}"#;
        assert!(matches!(
            parse_tool_call(ambiguous, &w, 4096),
            Err(DecodeError::UngrantedTool { .. })
        ));
        let leaf = br#"{"tool_call":{"name":"reverse","version":"","args":{"q":"x"}}}"#;
        let call = parse_tool_call(leaf, &w, 4096)
            .unwrap()
            .expect("the distinct leaf resolves to its unique grant");
        assert_eq!(call.name, ToolName("mcp-echo/reverse".into()));
        assert_eq!(call.version, ToolVersion("2".into()));
    }

    #[test]
    fn think_only_no_json_is_normal_completion() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Reasoning then prose (no JSON) ⇒ not a tool call.
        let env = b"<think>hmm</think>\nThe answer is blue.";
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn unclosed_think_is_normal_completion() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // An unterminated reasoning block strips to "" ⇒ fail-closed to None.
        let env = b"<think>reasoning with no closing tag and no json";
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn think_does_not_enable_midstring_injection() {
        // A `<think>` block whose body contains a JSON-looking object must NOT
        // be parsed as the call — only what FOLLOWS `</think>` is considered,
        // and here that's prose ⇒ None (the strict starts_with('{') gate holds).
        let w = warrant_granting(Some(("mcp-danger", "1")));
        let env = b"<think>{\"tool_call\":{\"name\":\"mcp-danger\",\"version\":\"1\",\"args\":{}}}</think> nope";
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn garbled_envelope_is_malformed_not_silently_dropped() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Started as a JSON object (committed to a call) but truncated → fail-closed.
        let env = br#"{"tool_call":{"name":"mcp-echo","version":"#;
        assert!(matches!(
            parse_tool_call(env, &w, 4096),
            Err(DecodeError::Malformed { .. })
        ));
    }

    #[test]
    fn trailing_garbage_after_envelope_is_malformed() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"tool_call":{"name":"mcp-echo","version":"1","args":{}}} then prose"#;
        assert!(matches!(
            parse_tool_call(env, &w, 4096),
            Err(DecodeError::Malformed { .. })
        ));
    }

    #[test]
    fn ungranted_tool_is_refused() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Right shape, but names a tool/version not in the grant set.
        let env = br#"{"tool_call":{"name":"mcp-danger","version":"1","args":{}}}"#;
        assert!(matches!(
            parse_tool_call(env, &w, 4096),
            Err(DecodeError::UngrantedTool { .. })
        ));
        // Same name, wrong version ⇒ also ungranted (exact match, SN-8).
        let env2 = br#"{"tool_call":{"name":"mcp-echo","version":"2","args":{}}}"#;
        assert!(matches!(
            parse_tool_call(env2, &w, 4096),
            Err(DecodeError::UngrantedTool { .. })
        ));
    }

    #[test]
    fn oversize_args_are_refused() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let big = "x".repeat(100);
        let env = format!(
            r#"{{"tool_call":{{"name":"mcp-echo","version":"1","args":{{"q":"{big}"}}}}}}"#
        );
        assert!(matches!(
            parse_tool_call(env.as_bytes(), &w, 8),
            Err(DecodeError::Oversize { .. })
        ));
    }

    #[test]
    fn non_utf8_is_a_normal_completion_not_a_panic() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        assert_eq!(parse_tool_call(&[0xff, 0xfe, 0x00], &w, 4096), Ok(None));
    }

    #[test]
    fn args_bytes_are_byte_identical_to_the_envelope_substring() {
        // PR-2d-1 pin: the decoded args are the EXACT bytes of the envelope's
        // args object — no re-serialization, no normalization (RawValue carry).
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let args_src = r#"{"q":"x","n":  7,"nested":{"a":[1,2,3]}}"#;
        let env =
            format!(r#"{{"tool_call":{{"name":"mcp-echo","version":"1","args":{args_src}}}}}"#);
        let call = parse_tool_call(env.as_bytes(), &w, 4096)
            .unwrap()
            .expect("a call");
        assert_eq!(call.args_bytes, args_src.as_bytes().to_vec());
    }

    // ---- BUG-28: Gemma-4 native `<|tool_call>call:NAME{ARGS}<tool_call|>` arm ----

    #[test]
    fn gemma_native_call_is_decoded_name_normalized_version_resolved() {
        let w = warrant_granting(Some(("fs-list", "1")));
        // Gemma's native syntax: underscore name, no version, empty args.
        let env = b"<|tool_call>call:fs_list{}<tool_call|>";
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native call");
        assert_eq!(call.name, ToolName("fs-list".into())); // `_`→`-` normalized
        assert_eq!(call.version, ToolVersion("1".into())); // resolved from the grant
        assert_eq!(call.args_bytes, b"{}".to_vec());
    }

    #[test]
    fn gemma_native_call_with_args_carries_bytes_verbatim() {
        let w = warrant_granting(Some(("fs-list", "1")));
        let env = br#"<|tool_call>call:fs_list{"path":"sub"}<tool_call|>"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native call");
        assert_eq!(call.args_bytes, br#"{"path":"sub"}"#.to_vec());
    }

    #[test]
    fn gemma_native_tolerates_missing_close_delim() {
        let w = warrant_granting(Some(("fs-list", "1")));
        // Truncated close delimiter — brace-balancing still bounds the args object.
        let env = br#"<|tool_call>call:fs_list{"path":"x"}"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native call");
        assert_eq!(call.args_bytes, br#"{"path":"x"}"#.to_vec());
    }

    #[test]
    fn gemma_native_after_channel_reasoning_is_decoded() {
        let w = warrant_granting(Some(("fs-list", "1")));
        // The `<|channel>` reasoning block is stripped first, then the native shape.
        let env = b"<|channel>thinking<channel|><|tool_call>call:fs_list{}<tool_call|>";
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native call");
        assert_eq!(call.name, ToolName("fs-list".into()));
    }

    #[test]
    fn gemma_native_ungranted_tool_is_refused() {
        let w = warrant_granting(Some(("fs-list", "1")));
        let env = b"<|tool_call>call:rm_rf{}<tool_call|>";
        assert!(matches!(
            parse_tool_call(env, &w, 4096),
            Err(DecodeError::UngrantedTool { .. })
        ));
    }

    #[test]
    fn gemma_native_empty_grants_is_none() {
        let w = warrant_granting(None); // step (0) short-circuits before any parse
        let env = b"<|tool_call>call:fs_list{}<tool_call|>";
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn gemma_native_oversize_args_refused() {
        let w = warrant_granting(Some(("fs-list", "1")));
        let big = "x".repeat(100);
        let env = format!(r#"<|tool_call>call:fs_list{{"p":"{big}"}}<tool_call|>"#);
        assert!(matches!(
            parse_tool_call(env.as_bytes(), &w, 8),
            Err(DecodeError::Oversize { .. })
        ));
    }

    #[test]
    fn gemma_native_overdeep_args_falls_through_to_none() {
        let w = warrant_granting(Some(("fs-list", "1")));
        let deep = format!(
            "<|tool_call>call:fs_list{}{}",
            "{".repeat(80),
            "}".repeat(80)
        );
        // balanced_object returns None past MAX_DEPTH ⇒ not a native call ⇒ falls to
        // the JSON gate, which sees a non-`{` start ⇒ Ok(None) (fail-closed).
        assert_eq!(parse_tool_call(deep.as_bytes(), &w, 4096), Ok(None));
    }

    #[test]
    fn gemma_native_no_brace_is_not_a_call() {
        let w = warrant_granting(Some(("fs-list", "1")));
        // Open delim but no `{` ⇒ not extractable ⇒ falls through ⇒ Ok(None).
        let env = b"<|tool_call>call:fs_list no args here";
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    // ---- BUG-32: namespace-strip + version-drift resolution (lane-agnostic) ----

    /// Grant several tools at once (the namespaced dialed-tool case the single-grant
    /// `warrant_granting` cannot express).
    fn warrant_granting_many(tools: &[(&str, &str)]) -> WarrantSpec {
        let mut w = warrant_granting(None);
        for (id, ver) in tools {
            w.tool_grants.insert(ToolGrant {
                tool_id: ToolName((*id).into()),
                tool_version: ToolVersion((*ver).into()),
            });
        }
        w
    }

    #[test]
    fn bug32_native_bare_leaf_resolves_namespaced_grant() {
        // The headline BUG-32 shape: a dialed/local tool is granted NAMESPACED, the
        // model proposes the bare leaf. The leaf must resolve to the namespaced grant.
        let w = warrant_granting_many(&[("kxlocal-a1b2c3d4/multiply", "1")]);
        let env = br#"<|tool_call>call:multiply{"a":2,"b":3}<tool_call|>"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native call");
        assert_eq!(call.name, ToolName("kxlocal-a1b2c3d4/multiply".into()));
        assert_eq!(call.version, ToolVersion("1".into()));
        assert_eq!(call.args_bytes, br#"{"a":2,"b":3}"#.to_vec());
    }

    #[test]
    fn bug32_envelope_bare_leaf_versionless_resolves_namespaced_grant() {
        // Same shape via the JSON envelope, with the empty version a model emits when
        // it copied the leaf rather than the full `tool.<id>@<ver>` label.
        let w = warrant_granting_many(&[("kxlocal-a1b2c3d4/multiply", "1")]);
        let env = br#"{"tool_call":{"name":"multiply","version":"","args":{"a":2}}}"#;
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("a call");
        assert_eq!(call.name, ToolName("kxlocal-a1b2c3d4/multiply".into()));
        assert_eq!(call.version, ToolVersion("1".into()));
    }

    #[test]
    fn bug32_envelope_separator_version_drift_resolves() {
        // The LIVE Gemma-4-12B shape from PR-9b-2b: the model emitted `mcp-echo:echo`
        // (the `<id>:<remote>` join) with an empty version against the `mcp-echo@1`
        // grant. The `:remote` tail is dropped, the head resolves, the grant's
        // version is taken. (Leaf-on-`/` alone would MISS this — there is no `/`.)
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"tool_call":{"name":"mcp-echo:echo","version":"","args":{"q":"x"}}}"#;
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("a call");
        assert_eq!(call.name, ToolName("mcp-echo".into()));
        assert_eq!(call.version, ToolVersion("1".into()));
        assert_eq!(call.args_bytes, br#"{"q":"x"}"#.to_vec());
    }

    #[test]
    fn bug32_ambiguous_leaf_is_fail_closed() {
        // Two distinct grants sharing the leaf `run` ⇒ the bare `run` is ambiguous ⇒
        // refused (SN-8: never guess which tool the model meant).
        let w = warrant_granting_many(&[("svc-a/run", "1"), ("svc-b/run", "1")]);
        let env = b"<|tool_call>call:run{}<tool_call|>";
        assert!(matches!(
            parse_tool_call(env, &w, 4096),
            Err(DecodeError::UngrantedTool { .. })
        ));
    }

    #[test]
    fn bug32_exact_full_id_still_wins_byte_identical() {
        // An exact full-id call against a namespaced grant resolves to itself — the
        // exact branch is preserved even though the leaf alias also exists.
        let w = warrant_granting_many(&[("kxlocal-a1b2c3d4/multiply", "1")]);
        let env = br#"{"tool_call":{"name":"kxlocal-a1b2c3d4/multiply","version":"1","args":{}}}"#;
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("a call");
        assert_eq!(call.name, ToolName("kxlocal-a1b2c3d4/multiply".into()));
        assert_eq!(call.version, ToolVersion("1".into()));
    }

    #[test]
    fn bug32_nonempty_wrong_version_still_refused() {
        // A NON-empty mismatching version is the model pinning a DIFFERENT tool —
        // stays refused (no version-recovery for non-empty versions; SN-8). Pins the
        // tightening that keeps `ungranted_tool_is_refused`'s @2 assertion valid.
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"tool_call":{"name":"mcp-echo","version":"2","args":{}}}"#;
        assert!(matches!(
            parse_tool_call(env, &w, 4096),
            Err(DecodeError::UngrantedTool { .. })
        ));
    }

    #[test]
    fn bug32_leaf_of_non_granted_tool_is_refused() {
        // A leaf that addresses NO grant ⇒ refused — the candidate set is exactly
        // `tool_grants`; a model cannot conjure a tool by naming a plausible leaf.
        let w = warrant_granting_many(&[("safe/list", "1")]);
        let env = b"<|tool_call>call:delete{}<tool_call|>";
        assert!(matches!(
            parse_tool_call(env, &w, 4096),
            Err(DecodeError::UngrantedTool { .. })
        ));
    }

    #[test]
    fn bug32_colon_injection_resolves_to_head_only_no_escalation() {
        // `mcp-echo:rm-rf` drops the `:rm-rf` tail and resolves to the granted
        // `mcp-echo` — the injected segment never reaches a tool (the remote-name is
        // fixed by the registry, not selectable by the model).
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"tool_call":{"name":"mcp-echo:rm-rf","version":"","args":{}}}"#;
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("a call");
        assert_eq!(call.name, ToolName("mcp-echo".into()));
        assert_eq!(call.version, ToolVersion("1".into()));
    }

    #[test]
    fn bug32_at_in_name_field_cannot_override_grant_version() {
        // An `@version` baked into the NAME field (version field empty) drops to the
        // grant's version — the model cannot force a version it was not granted.
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"tool_call":{"name":"mcp-echo@999","version":"","args":{}}}"#;
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("a call");
        assert_eq!(call.version, ToolVersion("1".into()));
    }

    #[test]
    fn bug32_empty_core_name_is_refused() {
        // A name that canonicalizes to nothing (just a `:` decoration) addresses no
        // grant ⇒ refused, never a silent match.
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"tool_call":{"name":":echo","version":"","args":{}}}"#;
        assert!(matches!(
            parse_tool_call(env, &w, 4096),
            Err(DecodeError::UngrantedTool { .. })
        ));
    }

    // ---- PR-9c-1: dynamic multi-format envelopes (accept-side; common open set) ----
    // Llama `<|python_tag|>{…}` · Qwen3/Hermes `<tool_call>{…}</tool_call>` · args
    // under args|arguments|parameters · args as a pre-serialized JSON string. All
    // are ACCEPT-side, fail-closed, and flow through the SAME grant resolution (SN-8)
    // as every other arm — the envelope-side complement to PR-3's args-side JSON5
    // repair. The markerless bare `{name,arguments}` object + multiple-calls-per-turn
    // are DEFERRED (pinned to Ok(None) below).

    #[test]
    fn python_tag_call_with_parameters_alias_is_decoded() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Llama-3.1/3.2 native: `<|python_tag|>` + a `{"name","parameters"}` object.
        let env = br#"<|python_tag|>{"name":"mcp-echo","parameters":{"q":"x"}}"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a python_tag call");
        assert_eq!(call.name, ToolName("mcp-echo".into()));
        assert_eq!(call.version, ToolVersion("1".into())); // the GRANT's version
        assert_eq!(call.args_bytes, br#"{"q":"x"}"#.to_vec());
    }

    #[test]
    fn python_tag_name_normalized_and_resolved() {
        let w = warrant_granting(Some(("fs-list", "1")));
        let env = br#"<|python_tag|>{"name":"fs_list","arguments":{}}"#;
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("a call");
        assert_eq!(call.name, ToolName("fs-list".into())); // `_`→`-` normalized
        assert_eq!(call.args_bytes, b"{}".to_vec());
    }

    #[test]
    fn python_tag_bare_leaf_resolves_namespaced_grant() {
        let w = warrant_granting_many(&[("kxlocal-a1b2c3d4/multiply", "1")]);
        let env = br#"<|python_tag|>{"name":"multiply","parameters":{"a":2,"b":3}}"#;
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("a call");
        assert_eq!(call.name, ToolName("kxlocal-a1b2c3d4/multiply".into()));
        assert_eq!(call.args_bytes, br#"{"a":2,"b":3}"#.to_vec());
    }

    #[test]
    fn python_tag_non_json_body_is_normal_completion() {
        // Llama's `<|python_tag|>func.call(...)` (non-JSON) form is OUT OF SCOPE:
        // no `{` after the marker ⇒ no balanced object ⇒ falls through ⇒ Ok(None).
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = b"<|python_tag|>echo(\"x\")";
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn python_tag_ungranted_tool_is_refused() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"<|python_tag|>{"name":"mcp-danger","arguments":{}}"#;
        assert!(matches!(
            parse_tool_call(env, &w, 4096),
            Err(DecodeError::UngrantedTool { .. })
        ));
    }

    #[test]
    fn xml_tool_call_newline_wrapped_is_decoded() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Qwen3's native: `<tool_call>\n{"name","arguments"}\n</tool_call>`.
        let env = b"<tool_call>\n{\"name\":\"mcp-echo\",\"arguments\":{\"q\":\"x\"}}\n</tool_call>";
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("an xml call");
        assert_eq!(call.name, ToolName("mcp-echo".into()));
        assert_eq!(call.args_bytes, br#"{"q":"x"}"#.to_vec());
    }

    #[test]
    fn xml_tool_call_tolerates_missing_close_tag() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Truncated close tag — `balanced_object` still bounds the object.
        let env = br#"<tool_call>{"name":"mcp-echo","arguments":{"q":"x"}}"#;
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("a call");
        assert_eq!(call.args_bytes, br#"{"q":"x"}"#.to_vec());
    }

    #[test]
    fn xml_tool_call_does_not_collide_with_gemma_native() {
        // `<tool_call>` (no pipe) must NOT be mistaken for Gemma's `<|tool_call>`
        // (with pipe) — the delimiters are distinct and the Gemma arm runs first.
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"<tool_call>{"name":"mcp-echo","args":{"q":"x"}}</tool_call>"#;
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("a call");
        assert_eq!(call.args_bytes, br#"{"q":"x"}"#.to_vec());
    }

    #[test]
    fn marked_object_without_name_is_normal_completion() {
        // A marked but un-named object is not a recognizable call ⇒ falls through.
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"<tool_call>{"foo":"bar"}</tool_call>"#;
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn two_args_aliases_is_fail_closed() {
        // Both `args` and `arguments` present ⇒ ambiguous ⇒ not a call ⇒ Ok(None).
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env =
            br#"<tool_call>{"name":"mcp-echo","args":{"q":"x"},"arguments":{"q":"y"}}</tool_call>"#;
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn string_args_are_reparsed_in_marked_shape() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Some models emit the args as a pre-serialized JSON string.
        let env = br#"<tool_call>{"name":"mcp-echo","arguments":"{\"q\":\"x\"}"}</tool_call>"#;
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("a call");
        assert_eq!(call.args_bytes, br#"{"q":"x"}"#.to_vec());
    }

    #[test]
    fn string_args_are_reparsed_in_wrapped_envelope() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // The Kortecx `{"tool_call":{…}}` envelope with a STRING args value.
        let env = br#"{"tool_call":{"name":"mcp-echo","version":"1","args":"{\"q\":\"x\"}"}}"#;
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("a call");
        assert_eq!(call.args_bytes, br#"{"q":"x"}"#.to_vec());
    }

    #[test]
    fn python_tag_oversize_args_refused() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let big = "x".repeat(100);
        let env = format!(r#"<|python_tag|>{{"name":"mcp-echo","parameters":{{"q":"{big}"}}}}"#);
        assert!(matches!(
            parse_tool_call(env.as_bytes(), &w, 8),
            Err(DecodeError::Oversize { .. })
        ));
    }

    #[test]
    fn marked_shape_empty_grants_is_none() {
        let w = warrant_granting(None); // step (0) short-circuits before any arm
        let env = br#"<|python_tag|>{"name":"mcp-echo","parameters":{}}"#;
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    // ---- PR-R1: COMMITMENT-AWARE markerless shapes — fire on a granted name +
    //      explicit args, degrade to a normal completion otherwise (no false-positive
    //      refusal); a MULTI-element `tool_calls` array stays DEFERRED. ----

    #[test]
    fn bare_function_object_with_granted_name_fires() {
        // Markerless `{"name":…,"arguments":…}` (Hermes/OpenAI): the name resolves to
        // a grant + an explicit args bag is present ⇒ FIRES (the same authority gate
        // as every other arm; only envelope recognition widened).
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"name":"mcp-echo","arguments":{"q":"x"}}"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a markerless call");
        assert_eq!(call.name, ToolName("mcp-echo".into()));
        assert_eq!(call.version, ToolVersion("1".into()));
        assert_eq!(call.args_bytes, br#"{"q":"x"}"#);
    }

    #[test]
    fn bare_object_with_ungranted_name_is_normal_completion() {
        // ADVERSARIAL (SN-8): a markerless object has NO commitment marker, so a name
        // that addresses no grant is PROSE, never a refusal — Ok(None), not UngrantedTool.
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"name":"not-a-tool","arguments":{"q":"x"}}"#;
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn bare_object_without_args_key_is_normal_completion() {
        // ADVERSARIAL: a bare object with ONLY a `name` and no args alias is far
        // likelier prose than a call (the markerless path requires an explicit args
        // bag) ⇒ Ok(None), even when the name happens to match a grant.
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"name":"mcp-echo"}"#;
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
        // A JSON object that merely carries a `name` key (prose) never fires.
        let prose = br#"{"name":"Ada Lovelace","born":1815}"#;
        assert_eq!(parse_tool_call(prose, &w, 4096), Ok(None));
    }

    #[test]
    fn bare_object_with_two_args_aliases_is_normal_completion() {
        // ADVERSARIAL: two args aliases ⇒ ambiguous ⇒ fail-closed (Ok(None)).
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"name":"mcp-echo","args":{"q":"x"},"arguments":{"q":"y"}}"#;
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn bare_object_oversize_args_refused() {
        // A markerless call's args are still size-capped (IMP-16) — a committed call.
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let big = "x".repeat(100);
        let env = format!(r#"{{"name":"mcp-echo","arguments":{{"q":"{big}"}}}}"#);
        assert!(matches!(
            parse_tool_call(env.as_bytes(), &w, 8),
            Err(DecodeError::Oversize { .. })
        ));
    }

    #[test]
    fn tool_calls_single_wrapper_fires() {
        // The OpenAI plural `{"tool_calls":[ <single> ]}` wrapper with one call FIRES.
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"tool_calls":[{"name":"mcp-echo","arguments":{"q":"x"}}]}"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a single tool_calls wrapper call");
        assert_eq!(call.name, ToolName("mcp-echo".into()));
        assert_eq!(call.version, ToolVersion("1".into()));
    }

    #[test]
    fn tool_calls_multi_element_array_is_deferred() {
        // DEFERRED: multiple-tool-calls-per-turn is a coordinator loop-semantics change
        // (one Tool fact/turn). A multi-element wrapper yields Ok(None) — NO silent
        // first-element cap (GR: no silent caps).
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"tool_calls":[{"name":"mcp-echo","arguments":{}},{"name":"mcp-echo","arguments":{}}]}"#;
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn tool_calls_empty_array_is_normal_completion() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"tool_calls":[]}"#;
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn multiple_tool_calls_array_is_deferred_normal_completion() {
        // A bare `[…]` array never passes the `{`-gate ⇒ Ok(None) (fires nothing).
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"[{"name":"mcp-echo","arguments":{}},{"name":"mcp-echo","arguments":{}}]"#;
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn markerless_shape_empty_grants_is_none() {
        // Step (0) short-circuits before ANY arm when the warrant grants no tools.
        let w = warrant_granting(None);
        let env = br#"{"name":"mcp-echo","arguments":{"q":"x"}}"#;
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn existing_shapes_a_and_b_unchanged_smoke() {
        // Re-assert both pre-existing shapes still decode after the widening.
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let a = br#"{"tool_call":{"name":"mcp-echo","version":"1","args":{"q":"x"}}}"#;
        assert!(parse_tool_call(a, &w, 4096).unwrap().is_some());
        let wf = warrant_granting(Some(("fs-list", "1")));
        let b = b"<|tool_call>call:fs_list{}<tool_call|>";
        assert!(parse_tool_call(b, &wf, 4096).unwrap().is_some());
    }
}
