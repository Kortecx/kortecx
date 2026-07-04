//! The fail-closed parse: [`parse_tool_call`] + the args-size cap
//! [`max_args_bytes`]. Moved verbatim from `kx-model-harness::toolcall`
//! (PR-2d-1); the 13 gate tests moved with it and pin the behavior.

use std::borrow::Cow;

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

/// Unwrap the union ANSWER arm `{"answer":"<text>"}` to its inner text, returning the
/// bytes VERBATIM (`Cow::Borrowed`) for anything else — prose, a `tool_call` envelope, or
/// any non-answer JSON. This is the presentation/commit companion of the Ollama non-strict
/// UNION `format` (`kx_grammar::ToolEnvelopeSpec::to_ollama_union_format`): when the union
/// is armed the model settles by emitting `{"answer":"…"}` instead of free prose, so the
/// display/commit layer unwraps it to the plain text a user expects.
///
/// PRESENTATION ONLY (SN-8) — never an authority decision — and a byte-identical NO-OP for
/// every non-union path (llama.cpp, non-tool turns, the canonical demo), so those commits
/// stay unchanged (`Cow::Borrowed`). `deny_unknown_fields` means only an EXACT single-key
/// `{"answer":<string>}` object unwraps; a stray field, a `tool_call`, prose, or non-JSON
/// all pass through untouched. This does NOT change parse CLASSIFICATION — the parser
/// already treats a non-`tool_call` object as `Ok(None)` (a settle).
#[must_use]
pub fn extract_answer(bytes: &[u8]) -> Cow<'_, [u8]> {
    // Only an EXACT single-key `{"answer":<string>}` object unwraps (`deny_unknown_fields`).
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct AnswerArm {
        answer: String,
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Cow::Borrowed(bytes);
    };
    let trimmed = text.trim();
    if !trimmed.starts_with('{') {
        return Cow::Borrowed(bytes);
    }
    match serde_json::from_str::<AnswerArm>(trimmed) {
        Ok(arm) => Cow::Owned(arm.answer.into_bytes()),
        Err(_) => Cow::Borrowed(bytes),
    }
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

/// Parse a model's listwise-rerank output (RC4c) into a validated PERMUTATION of
/// `[0, n)` — the retrieved candidate indices in the model's proposed best→worst
/// order.
///
/// **FAIL-CLOSED.** Returns `None` unless the extracted text is a JSON array of
/// EXACTLY `n` integers that is a permutation of `[0, n)` (length `n`, every element
/// in range, no duplicates). The grammar (`kx_grammar::PermutationSpec`) narrows the
/// SHAPE; this function is the AUTHORITY on validity (SN-8: the model proposes an
/// order, the runtime enforces exact validity — there is no similarity/closeness
/// operator). On `None`, the caller keeps the upstream (RRF/MMR) order, so a rerank
/// can never reorder into garbage.
///
/// Reuses the tool-call extractor: strips a single leading reasoning block
/// (`<think>…</think>` / `<|channel>…<channel|>`) and a surrounding markdown code
/// fence, then parses the LEADING JSON array and ignores any trailing bytes — a model
/// may append an explanation after the permutation (`[2,0,1] because …`). Deserializing
/// from position 0 keeps the SN-8 discipline (a JSON value boundary at the START, never
/// a mid-string scan); trailing content is discarded, not searched. Total + panic-free.
#[must_use]
pub fn parse_permutation(text: &str, n: usize) -> Option<Vec<usize>> {
    let body = extract_json_envelope(text);
    // Parse ONE leading value; a `StreamDeserializer` reads a single `Vec<i64>` from the
    // start and leaves any trailing bytes untouched (we never advance the iterator again).
    let arr: Vec<i64> = serde_json::Deserializer::from_str(body)
        .into_iter::<Vec<i64>>()
        .next()?
        .ok()?;
    if arr.len() != n {
        return None;
    }
    let mut seen = vec![false; n];
    let mut out = Vec::with_capacity(n);
    for v in arr {
        let idx = usize::try_from(v).ok()?;
        if idx >= n || seen[idx] {
            return None; // out of range OR duplicate ⇒ not a permutation
        }
        seen[idx] = true;
        out.push(idx);
    }
    Some(out)
}

/// Gemma-4's NATIVE tool-call open delimiter (`<|tool_call>call:NAME{ARGS}<tool_call|>`).
const GEMMA_TOOL_OPEN: &str = "<|tool_call>";
/// Gemma-4's NATIVE tool-call CLOSE delimiter — optional + truncation-tolerant for a
/// SINGLE call, but consumed between segments when a model emits a BATCH of native
/// calls back-to-back (T-MULTI-ELEMENT-TOOLCALLS).
const GEMMA_TOOL_CLOSE: &str = "<tool_call|>";
/// The optional `call:` marker after the open delimiter (observed: `call:fs_list{}`).
const GEMMA_CALL_MARKER: &str = "call:";

/// A model-NATIVE (non-envelope) call shape, post-extraction: the raw tool name
/// and the args-object bytes. The args are BORROWED for the brace form
/// (`NAME{…}`, the verbatim object) and OWNED for the paren form (`NAME(…)`, a
/// JSON object built from kwargs — T-GEMMA-PAREN). The version is resolved against
/// the grant set by the caller (Gemma emits no version).
struct NativeCall<'a> {
    raw_name: &'a str,
    args: Cow<'a, str>,
}

/// Extract a native call body from the text right AFTER the optional `call:`
/// marker. NAME runs up to the FIRST `{` or `(` (whichever is EARLIER — a DEFINED
/// boundary, never a mid-string scan, so the SN-8 injection boundary is unchanged:
/// only bytes the model fenced inside `<|tool_call>…` are promoted). The args are:
/// the brace-balanced `{…}` object (BORROWED, verbatim), OR the paren `(…)` body
/// converted to a JSON object (OWNED — T-GEMMA-PAREN). Returns the name, the args,
/// and the bytes consumed from `after_marker` (NAME + args span) so a batch scan
/// can advance. `None` on no boundary / empty name / unbounded args / unparseable
/// parens. Total + panic-free.
fn native_body(after_marker: &str) -> Option<(&str, Cow<'_, str>, usize)> {
    let brace = after_marker.find('{');
    let paren = after_marker.find('(');
    let (boundary, is_paren) = match (brace, paren) {
        // Brace wins when it is the EARLIER boundary (or the only one).
        (Some(b), Some(p)) if b <= p => (b, false),
        (Some(b), None) => (b, false),
        // Otherwise a paren boundary (brace absent, or brace after the paren).
        (Some(_) | None, Some(p)) => (p, true),
        (None, None) => return None,
    };
    let raw_name = after_marker[..boundary].trim();
    if raw_name.is_empty() {
        return None;
    }
    let region = &after_marker[boundary..];
    if is_paren {
        let span = balanced_parens(region)?;
        // Strip the outer parens; convert the kwargs/JSON body to a JSON object.
        let json = parse_paren_args(&span[1..span.len() - 1])?;
        Some((raw_name, Cow::Owned(json), boundary + span.len()))
    } else {
        let obj = balanced_object(region)?;
        let consumed = boundary + obj.len();
        Some((raw_name, Cow::Borrowed(obj), consumed))
    }
}

/// Extract a Gemma-4 native `<|tool_call>call:NAME{ARGS}<tool_call|>` (or the
/// `call:NAME(ARGS)` paren form, T-GEMMA-PAREN) call from the (reasoning-stripped)
/// text, or `None` if the text is not this shape. The `<tool_call|>` close is
/// optional (truncation-tolerant). Total + panic-free.
fn extract_gemma_native(text: &str) -> Option<NativeCall<'_>> {
    let after_open = text.trim_start().strip_prefix(GEMMA_TOOL_OPEN)?;
    let after_marker_ws = after_open.trim_start();
    let after_marker = after_marker_ws
        .strip_prefix(GEMMA_CALL_MARKER)
        .unwrap_or(after_marker_ws);
    let (raw_name, args, _consumed) = native_body(after_marker)?;
    Some(NativeCall { raw_name, args })
}

/// Gemma-4 sometimes wraps the FULL `{"tool_call":{…}}` ENVELOPE inside its native
/// delimiters — `<|tool_call>call:{"tool_call":{…}}<tool_call|>` — instead of the bare
/// `call:NAME{ARGS}`. The `NAME{ARGS}` arm ([`extract_gemma_native`]) reads an EMPTY name
/// there (the `{` follows the optional `call:`) and bails, so this returns the inner
/// brace-balanced `{…}` object for the envelope decoder to recover. DEFINED-delimiter
/// only (the marker is the boundary; [`balanced_object`] bounds the JSON) — never a
/// mid-string `{` search, so the SN-8 injection boundary is unchanged. `None` unless the
/// text opens with the Gemma marker AND the body is a leading brace-balanced object.
/// Total + panic-free.
fn gemma_marked_envelope(text: &str) -> Option<&str> {
    let after_open = text.trim_start().strip_prefix(GEMMA_TOOL_OPEN)?;
    let after_marker_ws = after_open.trim_start();
    let after_marker = after_marker_ws
        .strip_prefix(GEMMA_CALL_MARKER)
        .unwrap_or(after_marker_ws)
        .trim_start();
    if after_marker.starts_with('{') {
        balanced_object(after_marker)
    } else {
        None
    }
}

/// Resolve a decoded `{"tool_call":{name,version,args}}` envelope to a granted
/// [`ToolCall`], or a LOUD [`DecodeError`]. Exact `(name, version)` crypto-equality
/// FIRST (SN-8); only an empty version resolves the name to a UNIQUE grant (the menu-label
/// drift shape); a non-empty wrong version stays `UngrantedTool`. Args carried verbatim,
/// size-capped (IMP-16). Shared by the bare-envelope path AND the Gemma-marked-envelope
/// recovery so the authority surface is identical. Total + panic-free.
fn resolve_envelope_call(
    raw: RawToolCall,
    warrant: &WarrantSpec,
    max_args_bytes: usize,
) -> Result<ToolCall, DecodeError> {
    let name = ToolName(raw.name);
    let version = ToolVersion(raw.version);
    let exact = ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    };
    let grant = if warrant.tool_grants.contains(&exact) {
        exact
    } else if version.0.trim().is_empty() {
        match resolve_name(&name.0, warrant) {
            NameResolution::Unique(g) => g,
            NameResolution::Ambiguous(candidates) => {
                return Err(DecodeError::Ambiguous { name, candidates })
            }
            NameResolution::Unresolved => return Err(DecodeError::UngrantedTool { name, version }),
        }
    } else {
        return Err(DecodeError::UngrantedTool { name, version });
    };
    let args_bytes =
        args_value_bytes(&raw.args).unwrap_or_else(|| raw.args.get().as_bytes().to_vec());
    if args_bytes.len() > max_args_bytes {
        return Err(DecodeError::Oversize {
            got: args_bytes.len(),
            max: max_args_bytes,
        });
    }
    Ok(ToolCall {
        name: grant.tool_id,
        version: grant.tool_version,
        args_bytes,
    })
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

/// Return the prefix of `s` (which MUST start with `(`) spanning the first
/// paren-balanced `( … )` group, ignoring parens inside double-quoted strings
/// (with `\"` escapes). `None` if unbalanced or past `MAX_DEPTH`. The paren analog
/// of [`balanced_object`] (T-GEMMA-PAREN). Total + panic-free.
fn balanced_parens(s: &str) -> Option<&str> {
    const MAX_DEPTH: usize = 64;
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'(') {
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
            b'(' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return None;
                }
            }
            b')' => {
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

/// Split `s` on TOP-LEVEL commas — commas not inside a double-quoted string nor any
/// `()`/`[]`/`{}` nesting. Used to split paren kwargs (T-GEMMA-PAREN). Total +
/// panic-free.
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    let mut start = 0;
    for (i, b) in s.bytes().enumerate() {
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
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Parse a single kwargs value into ANY well-formed JSON value: a double-quoted
/// string, a number, `true`/`false`/`null`, OR a nested object/array (T-GEMMA-PAREN:
/// Gemma emits `call:NAME(config={"k":1}, tags=["a"])`). Single-quoted / bareword /
/// otherwise-malformed values ⇒ `None` (fail-closed — `serde_json` rejects non-JSON,
/// so the kwarg must be unambiguous JSON). `split_top_level_commas` already spans
/// `{}`/`[]` nesting, so a nested value arrives here intact. Total + panic-free.
fn parse_kw_value(v: &str) -> Option<serde_json::Value> {
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    // Accept any value `serde_json` can parse WHOLE (a trailing non-JSON tail makes
    // `from_str` fail ⇒ None), incl. nested objects/arrays. The produced map still
    // flows through `resolve_granted_name` + typed `validate_args` (SN-8 unchanged).
    serde_json::from_str(v).ok()
}

/// Convert Gemma's parenthesized native args (the bytes INSIDE `(…)`, T-GEMMA-PAREN)
/// into a JSON object STRING. Handles: empty `()` → `{}`; a wrapped JSON object
/// `({…})` → that object (whole-content only, nothing trailing); and comma-separated
/// `key=value` kwargs where each value is ANY well-formed JSON value (a scalar OR a
/// nested object/array — `key=val, cfg={"k":1}, tags=["a"]`). Fail-closed (`None`)
/// on positional args, unquoted/duplicate forms — an ambiguous shape falls through to
/// a normal completion rather than fabricating args. SN-8: the produced object still flows
/// through `resolve_granted_name` (exact grant) + the typed `validate_args`
/// downstream, so the authority surface is unchanged. Total + panic-free.
fn parse_paren_args(inner: &str) -> Option<String> {
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return Some("{}".to_string());
    }
    // A wrapped JSON object `({...})`: accept only if the object spans the WHOLE
    // content (nothing trailing) so we never silently drop bytes.
    if trimmed.starts_with('{') {
        let obj = balanced_object(trimmed)?;
        return (obj.len() == trimmed.len()).then(|| obj.to_string());
    }
    // kwargs: `key=scalar, key2=scalar, ...`.
    let mut map = serde_json::Map::new();
    for pair in split_top_level_commas(trimmed) {
        let (k, v) = pair.split_once('=')?;
        let key = k.trim();
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return None; // a key must be a plain identifier
        }
        let value = parse_kw_value(v)?;
        if map.insert(key.to_string(), value).is_some() {
            return None; // duplicate key ⇒ ambiguous ⇒ fail-closed
        }
    }
    if map.is_empty() {
        return None;
    }
    serde_json::to_string(&serde_json::Value::Object(map)).ok()
}

/// Llama-3.1/3.2's native tool-call open delimiter (`<|python_tag|>{"name":…}`).
const PYTHON_TAG_OPEN: &str = "<|python_tag|>";
/// Qwen3/Hermes XML-ish tool-call open tag (`<tool_call>{"name":…}</tool_call>`).
/// DISTINCT from Gemma's `<|tool_call>` (note the `|`): `strip_prefix` is exact, so
/// the two delimiters never collide, and the Gemma arm runs first.
const XML_TOOL_OPEN: &str = "<tool_call>";
/// Qwen3/Hermes XML-ish tool-call CLOSE tag — consumed between segments when a model
/// emits a BATCH of `<tool_call>{…}</tool_call><tool_call>{…}</tool_call>` calls
/// (T-MULTI-ELEMENT-TOOLCALLS). `<|python_tag|>` has no close delimiter.
const XML_TOOL_CLOSE: &str = "</tool_call>";

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

/// The outcome of resolving a model-emitted tool name against the grant set — the
/// SN-8-safe three-way distinction the callers need: a UNIQUE grant, NO grant, or an
/// AMBIGUOUS alias addressing ≥2 grants. Splitting ambiguity out (it used to collapse
/// into `None`) lets a COMMITTED arm raise [`DecodeError::Ambiguous`] with the
/// candidate full-ids so the react loop can re-prompt with a disambiguation, while a
/// markerless arm still degrades to a normal completion (T-CONNECTOR-AUTOGRANT).
enum NameResolution {
    /// Exactly one granted tool is addressed by the model's name core.
    Unique(ToolGrant),
    /// The name core addresses no grant (canon-empty, or no `id_matches` hit).
    Unresolved,
    /// The name core addresses ≥2 distinct grants — fail-closed (SN-8, no guessing).
    /// Carries the addressed full-ids in deterministic `tool_grants` order.
    Ambiguous(Vec<ToolName>),
}

/// Resolve a model-emitted (often separator-variant, version-less, or
/// namespace-stripped) tool name against the grant set, SN-8-safe. The match key is
/// the model's name core (its full id OR ANY `/`-segment, `canon`-normalized) via
/// [`id_matches`]. EXACT membership only — never a prefix/substring/fuzzy match. A
/// UNIQUE addressed grant ⇒ [`NameResolution::Unique`] (an element of
/// `warrant.tool_grants`, never widening the set); ≥2 ⇒ [`NameResolution::Ambiguous`]
/// (the candidate ids, in `BTreeSet` order); none ⇒ [`NameResolution::Unresolved`].
fn resolve_name(raw_name: &str, warrant: &WarrantSpec) -> NameResolution {
    let target = model_name_core(raw_name);
    if target.is_empty() {
        return NameResolution::Unresolved; // canonicalizes to nothing ⇒ addresses no grant
    }
    // `tool_grants` is a `BTreeSet`, so iteration (and thus the candidate vec) is in
    // deterministic `(tool_id, tool_version)` order — the refusal reason is reproducible.
    let hits: Vec<&ToolGrant> = warrant
        .tool_grants
        .iter()
        .filter(|g| id_matches(&target, &g.tool_id.0))
        .collect();
    match hits.as_slice() {
        [] => NameResolution::Unresolved,
        [g] => NameResolution::Unique((*g).clone()),
        many => NameResolution::Ambiguous(many.iter().map(|g| g.tool_id.clone()).collect()),
    }
}

/// The UNIQUE granted tool addressed by `raw_name`, or `None` for BOTH "no grant" and
/// "ambiguous" — the markerless caller's view, where neither is a refusal (a name that
/// does not resolve to exactly one grant is prose, not a committed call). COMMITTED
/// callers use [`resolve_name`] / [`committed_grant`] to distinguish the two.
fn resolve_granted_name(raw_name: &str, warrant: &WarrantSpec) -> Option<ToolGrant> {
    match resolve_name(raw_name, warrant) {
        NameResolution::Unique(grant) => Some(grant),
        NameResolution::Unresolved | NameResolution::Ambiguous(_) => None,
    }
}

/// Resolve a COMMITTED (marked/native/envelope) tool name to its unique grant, mapping
/// the two non-unique outcomes to the matching LOUD refusal: `Ambiguous` ⇒
/// [`DecodeError::Ambiguous`] (with the candidate full-ids for the disambiguating
/// re-prompt), no grant ⇒ [`DecodeError::UngrantedTool`] (version-less — the model
/// pinned no version on these arms). A marker IS the model's commitment, so a
/// non-unique name is never silent prose (unlike the markerless path).
fn committed_grant(raw_name: &str, warrant: &WarrantSpec) -> Result<ToolGrant, DecodeError> {
    match resolve_name(raw_name, warrant) {
        NameResolution::Unique(grant) => Ok(grant),
        NameResolution::Ambiguous(candidates) => Err(DecodeError::Ambiguous {
            name: ToolName(raw_name.to_string()),
            candidates,
        }),
        NameResolution::Unresolved => Err(DecodeError::UngrantedTool {
            name: ToolName(raw_name.to_string()),
            version: ToolVersion(String::new()),
        }),
    }
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

/// Resolve a MARKED/NATIVE (COMMITTED) call — a Gemma-native `NativeCall` — to a
/// granted `ToolCall`, fail-closed. A marker IS the model's commitment, so a name that
/// resolves to no grant is a LOUD `UngrantedTool` and a name that is ambiguous is a
/// LOUD `Ambiguous` (with candidate ids), never silent prose (unlike
/// [`markerless_call`]). Shared by the single Gemma arm of [`parse_tool_call`] and the
/// batch scan of [`parse_tool_calls`], so single + multi resolve identically.
fn resolve_native_call(
    native: &NativeCall<'_>,
    warrant: &WarrantSpec,
    max_args_bytes: usize,
) -> Result<ToolCall, DecodeError> {
    let grant = committed_grant(native.raw_name, warrant)?;
    let args_bytes = native.args.as_bytes().to_vec();
    if args_bytes.len() > max_args_bytes {
        return Err(DecodeError::Oversize {
            got: args_bytes.len(),
            max: max_args_bytes,
        });
    }
    Ok(ToolCall {
        name: grant.tool_id,
        version: grant.tool_version,
        args_bytes,
    })
}

/// Resolve a MARKED (COMMITTED) named call — the `(raw_name, args_bytes)` decoded
/// from a `<|python_tag|>` / `<tool_call>` object — to a granted `ToolCall`,
/// fail-closed (a no-grant name is a LOUD `UngrantedTool`, an ambiguous one a LOUD
/// `Ambiguous`). Shared by the single marked arm of [`parse_tool_call`] and the batch
/// scan of [`parse_tool_calls`].
fn resolve_marked_call(
    raw_name: &str,
    args_bytes: Vec<u8>,
    warrant: &WarrantSpec,
    max_args_bytes: usize,
) -> Result<ToolCall, DecodeError> {
    let grant = committed_grant(raw_name, warrant)?;
    if args_bytes.len() > max_args_bytes {
        return Err(DecodeError::Oversize {
            got: args_bytes.len(),
            max: max_args_bytes,
        });
    }
    Ok(ToolCall {
        name: grant.tool_id,
        version: grant.tool_version,
        args_bytes,
    })
}

/// T-MULTI-ELEMENT-TOOLCALLS: scan ALL back-to-back Gemma-native
/// `<|tool_call>call:NAME{ARGS}<tool_call|>` segments, in order. Each segment is
/// promoted ONLY after its DEFINED open delimiter (never a mid-string `{` search —
/// the SN-8 injection boundary is unchanged); `balanced_object` bounds each args
/// object so a close delim / the next segment can never leak in. The optional
/// `<tool_call|>` close between segments is consumed. Stops at the first byte that
/// does not open with the delimiter. Total + panic-free; a single segment yields a
/// 1-element vec (byte-identical to [`extract_gemma_native`]).
fn collect_gemma_calls(text: &str) -> Vec<NativeCall<'_>> {
    let mut out = Vec::new();
    let mut rest = text.trim_start();
    while let Some(after_open) = rest.strip_prefix(GEMMA_TOOL_OPEN) {
        let after_marker_ws = after_open.trim_start();
        let after_marker = after_marker_ws
            .strip_prefix(GEMMA_CALL_MARKER)
            .unwrap_or(after_marker_ws);
        let Some((raw_name, args, consumed)) = native_body(after_marker) else {
            break;
        };
        out.push(NativeCall { raw_name, args });
        // Advance past this segment (NAME + args span via `consumed`, a valid byte
        // index into `after_marker`), then a single optional close delimiter.
        rest = after_marker[consumed..].trim_start();
        if let Some(after_close) = rest.strip_prefix(GEMMA_TOOL_CLOSE) {
            rest = after_close.trim_start();
        }
    }
    out
}

/// T-MULTI-ELEMENT-TOOLCALLS: scan ALL back-to-back marked objects under a DEFINED
/// `open` delimiter (`<|python_tag|>` / `<tool_call>`), in order, consuming the
/// optional `close` tag between segments. Each object is the brace-balanced `{ … }`
/// following the marker (never a mid-string `{` search — SN-8 unchanged). Stops at
/// the first byte that does not open with `open`. Total + panic-free; a single
/// segment yields a 1-element vec (byte-identical to [`marked_object`]).
fn collect_marked_objects<'a>(text: &'a str, open: &str, close: Option<&str>) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = text.trim_start();
    while let Some(after_open) = rest.strip_prefix(open) {
        let after_ws = after_open.trim_start();
        let Some(obj) = balanced_object(after_ws) else {
            break;
        };
        out.push(obj);
        rest = after_ws[obj.len()..].trim_start();
        if let Some(c) = close {
            if let Some(after_close) = rest.strip_prefix(c) {
                rest = after_close.trim_start();
            }
        }
    }
    out
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

/// T-GEMMA-PAREN (markerless): the COMMITMENT-AWARE bare paren call — the WHOLE output
/// is `NAME(ARGS)` with NO marker and NO JSON wrapper (e.g. `refconn/echo(text="hi")`
/// or `reverse(text="pong")`), a shape some local models emit for a dialed
/// `<server>/<remote>` tool. The commitment signal (absent a marker) is that the ENTIRE
/// trimmed body IS the call — `NAME` + a single paren-balanced group spanning to the
/// end — so prose that merely mentions `foo(x)` is never mistaken for a call. Fires
/// ONLY when the name resolves to a UNIQUE grant AND `(…)` yields an explicit args bag
/// (via [`parse_paren_args`]); otherwise it degrades to a normal completion (`Ok(None)`
/// — never a false-positive refusal, matching [`markerless_call`]). The produced args
/// still flow through `resolve_granted_name` (exact grant) + the typed `validate_args`
/// downstream (SN-8 unchanged). Total + panic-free.
fn decode_markerless_paren(
    trimmed: &str,
    warrant: &WarrantSpec,
    max_args_bytes: usize,
) -> Result<Option<ToolCall>, DecodeError> {
    let Some(open) = trimmed.find('(') else {
        return Ok(None); // no paren ⇒ not this shape
    };
    let name = trimmed[..open].trim();
    // A real tool name is a single whitespace-free token (`refconn/echo`, `reverse`);
    // multi-word prose ("call the tool (now)") is not a markerless call.
    if name.is_empty() || name.split_whitespace().count() != 1 {
        return Ok(None);
    }
    let Some(paren) = balanced_parens(&trimmed[open..]) else {
        return Ok(None); // unbalanced ⇒ not a well-formed call
    };
    // The call must span the WHOLE trimmed body (nothing trailing) — the commitment
    // signal that distinguishes a bare call from prose containing `foo(x)`.
    if open + paren.len() != trimmed.len() {
        return Ok(None);
    }
    // The args are the bytes INSIDE the outer parens (`paren` includes both delimiters).
    let inner = &paren[1..paren.len() - 1];
    let Some(args_json) = parse_paren_args(inner) else {
        return Ok(None); // positional/ambiguous args ⇒ degrade to a normal completion
    };
    // Markerless posture: a name resolving to no grant OR an ambiguous one is prose,
    // not a refusal (`markerless_call` returns `Ok(None)` for both).
    markerless_call(name, args_json.into_bytes(), warrant, max_args_bytes)
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
        return Ok(Some(resolve_native_call(&native, warrant, max_args_bytes)?));
    }

    // (1a') Gemma-4 sometimes wraps the `{"tool_call":…}` ENVELOPE inside its native
    //       markers (`<|tool_call>call:{"tool_call":{…}}<tool_call|>`) rather than the
    //       bare `call:NAME{ARGS}`. The native arm above reads an EMPTY name there (the
    //       `{` follows the optional `call:`), so recover the inner envelope through the
    //       SAME exact-grant resolution + args cap as a bare envelope — SN-8 unchanged (the
    //       DEFINED marker is the boundary, never a `{` search). A wrapped object that is
    //       NOT a `tool_call` envelope falls through to a normal completion. (Live
    //       RC4b witness: Gemma-4 emits this for the `retrieve` tool — T-GEMMA-ENVELOPE-IN-MARKER.)
    if let Some(inner) = gemma_marked_envelope(trimmed) {
        if let Ok(Envelope {
            tool_call: Some(raw),
        }) = serde_json::from_str::<Envelope>(inner)
        {
            return Ok(Some(resolve_envelope_call(raw, warrant, max_args_bytes)?));
        }
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
        return Ok(Some(resolve_marked_call(
            &raw_name,
            args_bytes,
            warrant,
            max_args_bytes,
        )?));
    }

    // (1c) T-GEMMA-PAREN markerless: a non-JSON-object body that is the WHOLE call
    //      `NAME(ARGS)` (no marker, no wrapper). Fires only on a unique grant + explicit
    //      args; any other non-`{` body degrades to a normal completion (byte-identical
    //      to the prior bare early-return for every existing prose/non-call row).
    if !trimmed.starts_with('{') {
        return decode_markerless_paren(trimmed, warrant, max_args_bytes);
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

    // (3) The model committed to a tool call. Enforce tool ∈ warrant.tool_grants via the
    //     SHARED envelope resolver (exact (name, version) crypto-equality FIRST, SN-8;
    //     empty-version name-resolve for the menu-label drift shape; args carried verbatim
    //     + size-capped) — the SAME path the Gemma-marked-envelope recovery uses.
    Ok(Some(resolve_envelope_call(raw, warrant, max_args_bytes)?))
}

/// Decode ALL model-proposed tool calls from raw model output, fail-closed —
/// the multi-element (parallel tool-calling) complement to [`parse_tool_call`]
/// (T-MULTI-ELEMENT-TOOLCALLS).
///
/// Returns an ORDERED `Vec<ToolCall>` (the index is the `call_index` the coordinator
/// uses to disambiguate each observation): `[]` for a normal completion (prose,
/// non-envelope JSON, empty array, or any output under a no-grant warrant), `[c]`
/// for a single call (byte-identical to [`parse_tool_call`]'s `Ok(Some(c))`), and
/// `[c0, c1, …]` when the model emits N≥2 calls in one response — an `OpenAI`
/// `{"tool_calls":[…]}` array OR repeated marked/native segments
/// (`<|tool_call>…<|tool_call>…`, `<|python_tag|>…`×N, `<tool_call>…</tool_call>`×N).
///
/// Every call flows through the SAME grant resolution (exact `tool_grants`
/// membership, SN-8) + per-call args cap as the single decoder; the genuinely-multi
/// shapes are ALL-OR-NOTHING (a markerless array degrades the WHOLE body to a normal
/// completion if any element names no grant; a COMMITTED marked/native batch is a
/// LOUD `Err` if any segment names an ungranted tool). Total + panic-free.
///
/// # Errors
///
/// As [`parse_tool_call`] — [`DecodeError::Malformed`] / [`DecodeError::UngrantedTool`]
/// / [`DecodeError::Oversize`] — raised by the first offending call in a committed
/// envelope/marked batch.
pub fn parse_tool_calls(
    bytes: &[u8],
    warrant: &WarrantSpec,
    max_args_bytes: usize,
) -> Result<Vec<ToolCall>, DecodeError> {
    // Try the genuinely-multi shapes (≥2 calls) FIRST. If the output is not a
    // multi shape, fall back to the UNCHANGED single decoder — so every single-call
    // input decodes byte-identically (the same ToolCall the coordinator/harness
    // froze before this PR), preserving the react_shape ↔ harness golden equivalence.
    if let Some(calls) = try_decode_multi(bytes, warrant, max_args_bytes)? {
        return Ok(calls);
    }
    Ok(parse_tool_call(bytes, warrant, max_args_bytes)?
        .into_iter()
        .collect())
}

/// The multi-element (≥2 calls) detection that sits in front of the single decoder.
/// Returns `Ok(Some(vec))` when the output IS a multi shape (the vec is the decoded
/// batch — possibly empty for an all-or-nothing markerless degrade), `Ok(None)` when
/// it is NOT a multi shape (let [`parse_tool_call`] handle it), or `Err` when a
/// COMMITTED multi shape is malformed/ungranted/oversize. Total + panic-free; the
/// SN-8 boundary is the SAME defined-delimiter / `starts_with('{')` discipline as the
/// single path (no mid-string `{` search).
fn try_decode_multi(
    bytes: &[u8],
    warrant: &WarrantSpec,
    max_args_bytes: usize,
) -> Result<Option<Vec<ToolCall>>, DecodeError> {
    if warrant.tool_grants.is_empty() {
        return Ok(None);
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Ok(None);
    };
    let trimmed = extract_json_envelope(text);

    // (1a) Repeated Gemma-native segments. A COMMITTED batch ⇒ each segment resolves
    //      or is a loud refusal (mirrors the single Gemma arm).
    let gemma = collect_gemma_calls(trimmed);
    if gemma.len() >= 2 {
        let mut out = Vec::with_capacity(gemma.len());
        for native in &gemma {
            out.push(resolve_native_call(native, warrant, max_args_bytes)?);
        }
        return Ok(Some(out));
    }

    // (1b) Repeated python_tag / XML marked objects. A marked object that is not a
    //      recognizable named call ENDS the committed sequence (a trailing prose tail);
    //      a named one resolves or is a loud refusal.
    for (open, close) in [
        (PYTHON_TAG_OPEN, None),
        (XML_TOOL_OPEN, Some(XML_TOOL_CLOSE)),
    ] {
        let objs = collect_marked_objects(trimmed, open, close);
        if objs.len() >= 2 {
            let mut out = Vec::with_capacity(objs.len());
            for obj in objs {
                let Some((raw_name, args_bytes)) = decode_named_object(obj, false) else {
                    break; // a marked-but-not-named object ends the batch
                };
                out.push(resolve_marked_call(
                    &raw_name,
                    args_bytes,
                    warrant,
                    max_args_bytes,
                )?);
            }
            if out.len() >= 2 {
                return Ok(Some(out));
            }
            // <2 resolved ⇒ not a genuine batch; let the single decoder handle it.
        }
    }

    // (2) A `{"tool_calls":[…]}` wrapper with ≥2 elements (OpenAI / vLLM parallel
    //     calls). Markerless ⇒ ALL-OR-NOTHING: any element that names no grant OR is
    //     not a named-call object degrades the WHOLE body to a normal completion
    //     (no false-positive refusal, no silent first-element cap).
    if trimmed.starts_with('{') {
        #[derive(Deserialize)]
        struct ToolCalls<'a> {
            #[serde(borrow)]
            tool_calls: Vec<&'a RawValue>,
        }
        if let Ok(wrapper) = serde_json::from_str::<ToolCalls>(trimmed) {
            if wrapper.tool_calls.len() >= 2 {
                let mut out = Vec::with_capacity(wrapper.tool_calls.len());
                for raw in &wrapper.tool_calls {
                    let Some((raw_name, args_bytes)) = decode_named_object(raw.get(), true) else {
                        return Ok(Some(Vec::new())); // not a named call ⇒ whole body degrades
                    };
                    match markerless_call(&raw_name, args_bytes, warrant, max_args_bytes)? {
                        Some(call) => out.push(call),
                        None => return Ok(Some(Vec::new())), // ungranted name ⇒ prose (whole body)
                    }
                }
                return Ok(Some(out));
            }
        }
    }

    Ok(None)
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
        let err =
            parse_tool_call(ambiguous, &w, 4096).expect_err("shared server segment ⇒ refused");
        let DecodeError::Ambiguous { name, candidates } = err else {
            panic!("expected DecodeError::Ambiguous, got {err:?}");
        };
        assert_eq!(name, ToolName("mcp-echo".into()));
        assert_eq!(
            candidates,
            vec![
                ToolName("mcp-echo/echo".into()),
                ToolName("mcp-echo/reverse".into())
            ]
        );
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
    fn gemma_marked_envelope_is_recovered_to_a_granted_call() {
        // RC4b live-witness (T-GEMMA-ENVELOPE-IN-MARKER): Gemma-4 sometimes wraps the FULL
        // {"tool_call":{…}} envelope INSIDE its native markers (`call:` then a `{`, not a
        // bare name). The native NAME{ARGS} arm reads an EMPTY name; the envelope must be
        // recovered through the same exact-grant resolution + args cap (SN-8).
        let w = warrant_granting(Some(("retrieve", "1")));
        let env = br#"<|tool_call>call:{"tool_call":{"name":"retrieve","version":"1","args":{"dataset":"science","query":"plants energy sun"}}}<tool_call|>"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("the marked envelope recovers to a granted call");
        assert_eq!(call.name, ToolName("retrieve".into()));
        assert_eq!(call.version, ToolVersion("1".into()));
        assert_eq!(
            call.args_bytes,
            br#"{"dataset":"science","query":"plants energy sun"}"#.to_vec()
        );
    }

    #[test]
    fn gemma_marked_envelope_without_call_marker_is_recovered() {
        // The `call:` marker is optional — `<|tool_call>{envelope}` recovers too.
        let w = warrant_granting(Some(("retrieve", "1")));
        let env = br#"<|tool_call>{"tool_call":{"name":"retrieve","version":"1","args":{"dataset":"d","query":"q"}}}"#;
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("recovered");
        assert_eq!(call.name, ToolName("retrieve".into()));
    }

    #[test]
    fn gemma_marked_envelope_ungranted_is_refused_not_silent() {
        // SN-8: a marked envelope naming an UNGRANTED tool is a LOUD refusal, never prose —
        // the recovery still flows through the exact-grant authority gate.
        let w = warrant_granting(Some(("retrieve", "1")));
        let env = br#"<|tool_call>call:{"tool_call":{"name":"rm-rf","version":"1","args":{}}}<tool_call|>"#;
        assert!(matches!(
            parse_tool_call(env, &w, 4096),
            Err(DecodeError::UngrantedTool { .. })
        ));
    }

    #[test]
    fn gemma_marked_non_envelope_object_is_a_normal_completion() {
        // A Gemma marker wrapping a NON-tool_call object is prose, not a refusal (the
        // model emitted structured non-call JSON). Degrades to a normal completion.
        let w = warrant_granting(Some(("retrieve", "1")));
        let env = br#"<|tool_call>call:{"thought":"I should search"}<tool_call|>"#;
        assert!(parse_tool_call(env, &w, 4096).unwrap().is_none());
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

    // ---- T-GEMMA-PAREN: Gemma-4 native `<|tool_call>call:NAME(ARGS)` paren arm ----

    #[test]
    fn gemma_paren_kwargs_string_value_is_decoded() {
        let w = warrant_granting(Some(("fs-list", "1")));
        // Paren kwargs with a double-quoted string value → JSON object.
        let env = br#"<|tool_call>call:fs_list(path="sub")<tool_call|>"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native paren call");
        assert_eq!(call.name, ToolName("fs-list".into()));
        assert_eq!(call.version, ToolVersion("1".into()));
        assert_eq!(call.args_bytes, br#"{"path":"sub"}"#.to_vec());
    }

    #[test]
    fn gemma_paren_empty_args_is_empty_object() {
        let w = warrant_granting(Some(("fs-list", "1")));
        let env = b"<|tool_call>call:fs_list()<tool_call|>";
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native paren call");
        assert_eq!(call.args_bytes, b"{}".to_vec());
    }

    #[test]
    fn gemma_paren_multi_kwargs_with_scalars_is_decoded() {
        let w = warrant_granting(Some(("fs-list", "1")));
        // Mixed scalar kwargs (string, number, bool) — order preserved by the map.
        let env = br#"<|tool_call>call:fs_list(path="x", depth=2, recurse=true)<tool_call|>"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native paren call");
        let v: serde_json::Value = serde_json::from_slice(&call.args_bytes).unwrap();
        assert_eq!(v["path"], "x");
        assert_eq!(v["depth"], 2);
        assert_eq!(v["recurse"], true);
    }

    #[test]
    fn gemma_paren_wrapped_json_object_is_decoded() {
        let w = warrant_granting(Some(("fs-list", "1")));
        // A JSON object wrapped in parens: call:NAME({...}).
        let env = br#"<|tool_call>call:fs_list({"path":"sub"})<tool_call|>"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native paren call");
        assert_eq!(call.args_bytes, br#"{"path":"sub"}"#.to_vec());
    }

    #[test]
    fn gemma_paren_string_value_with_comma_stays_one_arg() {
        let w = warrant_granting(Some(("fs-list", "1")));
        // A comma INSIDE a quoted value must not split the kwargs.
        let env = br#"<|tool_call>call:fs_list(q="a,b")<tool_call|>"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native paren call");
        let v: serde_json::Value = serde_json::from_slice(&call.args_bytes).unwrap();
        assert_eq!(v["q"], "a,b");
        assert_eq!(v.as_object().unwrap().len(), 1);
    }

    #[test]
    fn gemma_paren_nested_object_kwarg_is_decoded() {
        // T-GEMMA-PAREN fix: a kwarg whose value is a NESTED JSON object.
        let w = warrant_granting(Some(("fs-list", "1")));
        let env = br#"<|tool_call>call:fs_list(cfg={"depth":2,"hidden":true})<tool_call|>"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native paren call");
        let v: serde_json::Value = serde_json::from_slice(&call.args_bytes).unwrap();
        assert_eq!(v["cfg"]["depth"], 2);
        assert_eq!(v["cfg"]["hidden"], true);
    }

    #[test]
    fn gemma_paren_array_kwarg_is_decoded() {
        // T-GEMMA-PAREN fix: a kwarg whose value is a JSON ARRAY (top-level commas
        // inside `[]` must not split the kwargs — `split_top_level_commas` spans it).
        let w = warrant_granting(Some(("fs-list", "1")));
        let env = br#"<|tool_call>call:fs_list(tags=["a","b","c"])<tool_call|>"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native paren call");
        let v: serde_json::Value = serde_json::from_slice(&call.args_bytes).unwrap();
        assert_eq!(v["tags"], serde_json::json!(["a", "b", "c"]));
    }

    #[test]
    fn gemma_paren_mixed_scalar_object_array_kwargs_decoded() {
        // The headline T-GEMMA-PAREN shape: a scalar + a nested object + an array.
        let w = warrant_granting(Some(("fs-list", "1")));
        let env = br#"<|tool_call>call:fs_list(path="x", cfg={"k":1}, tags=["a","b"])<tool_call|>"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native paren call");
        let v: serde_json::Value = serde_json::from_slice(&call.args_bytes).unwrap();
        assert_eq!(v["path"], "x");
        assert_eq!(v["cfg"]["k"], 1);
        assert_eq!(v["tags"], serde_json::json!(["a", "b"]));
        assert_eq!(v.as_object().unwrap().len(), 3);
    }

    #[test]
    fn gemma_paren_malformed_nested_value_falls_through_to_none() {
        // A kwarg value that is NOT whole JSON (a trailing tail) ⇒ fail-closed.
        let w = warrant_granting(Some(("fs-list", "1")));
        let env = br#"<|tool_call>call:fs_list(cfg={"k":1} junk)<tool_call|>"#;
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn gemma_paren_positional_or_bareword_falls_through_to_none() {
        let w = warrant_granting(Some(("fs-list", "1")));
        // Positional args (no `=`) ⇒ fail-closed (not fabricated) ⇒ Ok(None).
        let positional = b"<|tool_call>call:fs_list(\"sub\")<tool_call|>";
        assert_eq!(parse_tool_call(positional, &w, 4096), Ok(None));
        // A bareword (unquoted string) value ⇒ fail-closed ⇒ Ok(None).
        let bareword = b"<|tool_call>call:fs_list(path=sub)<tool_call|>";
        assert_eq!(parse_tool_call(bareword, &w, 4096), Ok(None));
    }

    #[test]
    fn gemma_paren_tolerates_missing_close_delim() {
        let w = warrant_granting(Some(("fs-list", "1")));
        // Truncated `<tool_call|>` — paren-balancing still bounds the args group.
        let env = br#"<|tool_call>call:fs_list(path="x")"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native paren call");
        assert_eq!(call.args_bytes, br#"{"path":"x"}"#.to_vec());
    }

    #[test]
    fn gemma_paren_batch_with_brace_call_both_fire() {
        // T-MULTI-ELEMENT: a paren call followed by a brace call — both fire.
        let w = warrant_granting_many(&[("fs-list", "1"), ("echo", "2")]);
        let env = br#"<|tool_call>call:fs_list(path="x")<tool_call|><|tool_call>call:echo{"q":"hi"}<tool_call|>"#;
        let calls = parse_tool_calls(env, &w, 4096).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, ToolName("fs-list".into()));
        assert_eq!(calls[0].args_bytes, br#"{"path":"x"}"#.to_vec());
        assert_eq!(calls[1].name, ToolName("echo".into()));
        assert_eq!(calls[1].args_bytes, br#"{"q":"hi"}"#.to_vec());
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
        // refused (SN-8: never guess which tool the model meant). The COMMITTED arm now
        // raises the precise `Ambiguous` variant carrying the candidate full-ids (in
        // BTreeSet order) so the react loop can re-prompt with a disambiguation
        // (T-CONNECTOR-AUTOGRANT-LIVE-DEADLETTER) — still fail-closed, never fires.
        let w = warrant_granting_many(&[("svc-a/run", "1"), ("svc-b/run", "1")]);
        let env = b"<|tool_call>call:run{}<tool_call|>";
        let err = parse_tool_call(env, &w, 4096).expect_err("ambiguous ⇒ refused");
        let DecodeError::Ambiguous { name, candidates } = err else {
            panic!("expected DecodeError::Ambiguous, got {err:?}");
        };
        assert_eq!(name, ToolName("run".into()));
        assert_eq!(
            candidates,
            vec![ToolName("svc-a/run".into()), ToolName("svc-b/run".into())]
        );
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
    fn tool_calls_multi_element_array_singular_defers() {
        // BACK-COMPAT: the SINGULAR `parse_tool_call` still defers a multi-element
        // wrapper to Ok(None) — NO silent first-element cap (the multi path is the
        // plural `parse_tool_calls`, tested below). A caller still on the singular sees
        // the historical "deferred" behavior, never a silent cap.
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"tool_calls":[{"name":"mcp-echo","arguments":{}},{"name":"mcp-echo","arguments":{}}]}"#;
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn parse_tool_calls_multi_element_array_fires_all() {
        // T-MULTI-ELEMENT-TOOLCALLS: the PLURAL decoder fires ALL N calls in order,
        // each grant-resolved with its args carried verbatim (the Vec index is the
        // call_index). No silent cap.
        let w = warrant_granting_many(&[("mcp-echo", "1"), ("fs-read", "1")]);
        let env = br#"{"tool_calls":[{"name":"mcp-echo","arguments":{"q":"x"}},{"name":"fs-read","arguments":{"p":"/a"}}]}"#;
        let calls = parse_tool_calls(env, &w, 4096).unwrap();
        assert_eq!(calls.len(), 2, "both calls fire");
        assert_eq!(calls[0].name, ToolName("mcp-echo".into()));
        assert_eq!(calls[0].args_bytes, br#"{"q":"x"}"#.to_vec());
        assert_eq!(calls[1].name, ToolName("fs-read".into()));
        assert_eq!(calls[1].args_bytes, br#"{"p":"/a"}"#.to_vec());
    }

    #[test]
    fn parse_tool_calls_same_tool_twice_fires_both() {
        // Two calls to the SAME tool with DIFFERENT args both fire (the observation
        // ids are disambiguated downstream by call_index, not here). No dedup.
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"tool_calls":[{"name":"mcp-echo","arguments":{"q":"x"}},{"name":"mcp-echo","arguments":{"q":"y"}}]}"#;
        let calls = parse_tool_calls(env, &w, 4096).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].args_bytes, br#"{"q":"x"}"#.to_vec());
        assert_eq!(calls[1].args_bytes, br#"{"q":"y"}"#.to_vec());
    }

    #[test]
    fn parse_tool_calls_single_is_byte_identical_to_singular() {
        // A single-call input through the plural decoder yields the SAME ToolCall the
        // singular returns — the byte-identical equivalence the react_shape ↔ harness
        // golden depends on.
        let w = warrant_granting(Some(("mcp-echo", "1")));
        for env in [
            br#"{"tool_call":{"name":"mcp-echo","version":"1","args":{"q":"x"}}}"#.as_slice(),
            br#"{"tool_calls":[{"name":"mcp-echo","arguments":{"q":"x"}}]}"#.as_slice(),
            br#"<|tool_call>call:mcp_echo{"q":"x"}<tool_call|>"#.as_slice(),
            br#"<tool_call>{"name":"mcp-echo","arguments":{"q":"x"}}</tool_call>"#.as_slice(),
        ] {
            let plural = parse_tool_calls(env, &w, 4096).unwrap();
            let singular = parse_tool_call(env, &w, 4096).unwrap();
            assert_eq!(plural.len(), 1, "single input ⇒ one call: {env:?}");
            assert_eq!(Some(plural[0].clone()), singular, "plural[0] == singular");
        }
    }

    #[test]
    fn parse_tool_calls_empty_array_is_completion() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        assert_eq!(
            parse_tool_calls(br#"{"tool_calls":[]}"#, &w, 4096).unwrap(),
            vec![]
        );
    }

    #[test]
    fn parse_tool_calls_array_with_one_ungranted_degrades_whole_body() {
        // ALL-OR-NOTHING markerless: a 2-element array where ONE name is ungranted
        // degrades the WHOLE body to a normal completion (no partial fire, no refusal).
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"tool_calls":[{"name":"mcp-echo","arguments":{"q":"x"}},{"name":"not-granted","arguments":{}}]}"#;
        assert_eq!(parse_tool_calls(env, &w, 4096).unwrap(), vec![]);
    }

    #[test]
    fn parse_tool_calls_repeated_gemma_native_segments_fire_all() {
        // Repeated Gemma-native `<|tool_call>…<tool_call|>` segments back-to-back fire
        // all N (the live-Gemma parallel-call shape). SN-8: each segment is promoted
        // ONLY after its defined open delimiter.
        let w = warrant_granting_many(&[("mcp-echo", "1"), ("fs-read", "1")]);
        let env = br#"<|tool_call>call:mcp_echo{"q":"x"}<tool_call|><|tool_call>call:fs_read{"p":"/a"}<tool_call|>"#;
        let calls = parse_tool_calls(env, &w, 4096).unwrap();
        assert_eq!(calls.len(), 2, "both native segments fire");
        assert_eq!(calls[0].name, ToolName("mcp-echo".into()));
        assert_eq!(calls[1].name, ToolName("fs-read".into()));
    }

    #[test]
    fn parse_tool_calls_repeated_xml_segments_fire_all() {
        // Repeated Qwen3/Hermes `<tool_call>{…}</tool_call>` segments fire all N.
        let w = warrant_granting_many(&[("mcp-echo", "1"), ("fs-read", "1")]);
        let env = br#"<tool_call>{"name":"mcp-echo","arguments":{"q":"x"}}</tool_call><tool_call>{"name":"fs-read","arguments":{"p":"/a"}}</tool_call>"#;
        let calls = parse_tool_calls(env, &w, 4096).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].name, ToolName("fs-read".into()));
    }

    #[test]
    fn parse_tool_calls_repeated_python_tag_segments_fire_all() {
        // Repeated Llama `<|python_tag|>{…}` markers fire all N.
        let w = warrant_granting_many(&[("mcp-echo", "1"), ("fs-read", "1")]);
        let env = br#"<|python_tag|>{"name":"mcp-echo","parameters":{"q":"x"}}<|python_tag|>{"name":"fs-read","parameters":{"p":"/a"}}"#;
        let calls = parse_tool_calls(env, &w, 4096).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, ToolName("mcp-echo".into()));
    }

    #[test]
    fn parse_tool_calls_committed_batch_with_ungranted_segment_is_refused() {
        // A COMMITTED marked/native batch with an ungranted name is a LOUD refusal
        // (mirrors the single-call marked commitment rule — a marker IS commitment).
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"<tool_call>{"name":"mcp-echo","arguments":{}}</tool_call><tool_call>{"name":"not-granted","arguments":{}}</tool_call>"#;
        assert!(matches!(
            parse_tool_calls(env, &w, 4096),
            Err(DecodeError::UngrantedTool { .. })
        ));
    }

    #[test]
    fn parse_tool_calls_no_grants_is_empty() {
        // The security default: no grants ⇒ no call can ever fire, even a multi body.
        let w = warrant_granting(None);
        let env = br#"{"tool_calls":[{"name":"mcp-echo","arguments":{}},{"name":"mcp-echo","arguments":{}}]}"#;
        assert_eq!(parse_tool_calls(env, &w, 4096).unwrap(), vec![]);
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

    // ---- T-CONNECTOR-AUTOGRANT-LIVE-DEADLETTER: dialed-connector collision +
    //      multi-format (the shapes a live model emits for a `<server>/<remote>` tool).

    /// The dialed-vs-bundled collision: the bundled `mcp-echo/echo` and a dialed
    /// `refconn/echo` are BOTH auto-granted, so a COMMITTED bare `echo` is ambiguous ⇒
    /// the precise `Ambiguous` refusal carrying both candidate full-ids (`BTreeSet` order)
    /// for the disambiguating re-prompt. This is the turn-0 dead-letter root cause.
    #[test]
    fn dialed_collision_bare_leaf_is_ambiguous_with_candidates() {
        let w = warrant_granting_many(&[
            ("mcp-echo/echo", "1"),
            ("refconn/echo", "1"),
            ("refconn/reverse", "1"),
        ]);
        let env = br#"<|tool_call>call:echo{"q":"pong"}<tool_call|>"#;
        let err = parse_tool_call(env, &w, 4096).expect_err("bare echo is ambiguous");
        let DecodeError::Ambiguous { name, candidates } = err else {
            panic!("expected Ambiguous, got {err:?}");
        };
        assert_eq!(name, ToolName("echo".into()));
        assert_eq!(
            candidates,
            vec![
                ToolName("mcp-echo/echo".into()),
                ToolName("refconn/echo".into())
            ],
            "only the two `echo` grants collide; `refconn/reverse` is unaddressed"
        );
    }

    /// The disambiguation the re-prompt steers toward: the FULL `<server>/<remote>` id
    /// resolves uniquely even when a colliding leaf exists.
    #[test]
    fn full_id_disambiguates_dialed_collision() {
        let w = warrant_granting_many(&[("mcp-echo/echo", "1"), ("refconn/echo", "1")]);
        let env = br#"<|tool_call>call:refconn/echo{"q":"pong"}<tool_call|>"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("the full id resolves uniquely");
        assert_eq!(call.name, ToolName("refconn/echo".into()));
        assert_eq!(call.version, ToolVersion("1".into()));
    }

    /// A server-prefix segment resolves when that server exposes exactly one granted
    /// tool (Gemma-4 sometimes emits the bare `<server>`).
    #[test]
    fn dialed_server_prefix_unique_resolves() {
        let w = warrant_granting_many(&[("mcp-echo/echo", "1"), ("solo/run", "1")]);
        let env = br#"<|tool_call>call:solo{"x":1}<tool_call|>"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("the unique server prefix resolves");
        assert_eq!(call.name, ToolName("solo/run".into()));
    }

    /// T-GEMMA-PAREN markerless: the WHOLE body is `NAME(kwargs)` for a dialed tool ⇒
    /// fires, with the kwargs lowered to a JSON args object.
    #[test]
    fn markerless_paren_dialed_full_id_fires() {
        let w = warrant_granting_many(&[("refconn/reverse", "1")]);
        let b = br#"refconn/reverse(text="hi")"#;
        let call = parse_tool_call(b, &w, 4096)
            .unwrap()
            .expect("markerless paren call fires");
        assert_eq!(call.name, ToolName("refconn/reverse".into()));
        assert_eq!(call.version, ToolVersion("1".into()));
        assert_eq!(call.args_bytes, br#"{"text":"hi"}"#.to_vec());
    }

    /// Markerless paren with a unique bare leaf resolves to its namespaced grant.
    #[test]
    fn markerless_paren_bare_leaf_fires() {
        let w = warrant_granting_many(&[("refconn/reverse", "1")]);
        let b = br#"reverse(text="hi")"#;
        let call = parse_tool_call(b, &w, 4096)
            .unwrap()
            .expect("bare-leaf paren resolves");
        assert_eq!(call.name, ToolName("refconn/reverse".into()));
    }

    /// A markerless paren naming no grant is a NORMAL completion (prose), never a
    /// false-positive refusal (the markerless posture).
    #[test]
    fn markerless_paren_ungranted_is_prose() {
        let w = warrant_granting_many(&[("refconn/reverse", "1")]);
        assert_eq!(parse_tool_call(br#"notatool(x="1")"#, &w, 4096), Ok(None));
    }

    /// A markerless paren whose name is AMBIGUOUS degrades to prose (markerless never
    /// refuses — only the COMMITTED arms raise `Ambiguous`).
    #[test]
    fn markerless_paren_ambiguous_is_prose() {
        let w = warrant_granting_many(&[("mcp-echo/echo", "1"), ("refconn/echo", "1")]);
        assert_eq!(parse_tool_call(br#"echo(q="pong")"#, &w, 4096), Ok(None));
    }

    /// Prose that merely MENTIONS a call (`reverse(...)` not spanning the whole body)
    /// is not a markerless call — the commitment signal is the whole body being it.
    #[test]
    fn markerless_paren_embedded_in_prose_is_not_a_call() {
        let w = warrant_granting_many(&[("refconn/reverse", "1")]);
        let b = br#"I will call reverse(text="hi") now."#;
        assert_eq!(parse_tool_call(b, &w, 4096), Ok(None));
    }

    /// A markerless paren whose lowered args overshoot the cap is a LOUD refusal (the
    /// resolved-then-oversize path, like every other arm).
    #[test]
    fn markerless_paren_oversize_refused() {
        let w = warrant_granting_many(&[("refconn/reverse", "1")]);
        let big = format!(r#"reverse(text="{}")"#, "x".repeat(100));
        assert!(matches!(
            parse_tool_call(big.as_bytes(), &w, 10),
            Err(DecodeError::Oversize { .. })
        ));
    }

    /// Multi-element parallel calls over a dialed connector fire ALL in order.
    #[test]
    fn multi_element_dialed_connector_fires_all() {
        let w = warrant_granting_many(&[("refconn/echo", "1"), ("refconn/reverse", "1")]);
        let b = br#"<|tool_call>call:refconn/echo{"q":"a"}<tool_call|><|tool_call>call:refconn/reverse{"text":"b"}<tool_call|>"#;
        let calls = parse_tool_calls(b, &w, 4096).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, ToolName("refconn/echo".into()));
        assert_eq!(calls[1].name, ToolName("refconn/reverse".into()));
    }

    // ── RC4c: parse_permutation (fail-closed listwise-rerank order) ─────────

    #[test]
    fn parse_permutation_accepts_a_valid_permutation() {
        assert_eq!(parse_permutation("[2,0,1]", 3), Some(vec![2, 0, 1]));
        assert_eq!(parse_permutation("[0]", 1), Some(vec![0]));
        // identity is valid.
        assert_eq!(parse_permutation("[0,1,2,3]", 4), Some(vec![0, 1, 2, 3]));
        // whitespace tolerated (the GBNF emits it).
        assert_eq!(parse_permutation("[ 1 , 0 ]", 2), Some(vec![1, 0]));
    }

    #[test]
    fn parse_permutation_strips_reasoning_and_code_fence() {
        assert_eq!(
            parse_permutation("<think>rank them</think>[1,0]", 2),
            Some(vec![1, 0])
        );
        assert_eq!(
            parse_permutation("```json\n[2,1,0]\n```", 3),
            Some(vec![2, 1, 0])
        );
    }

    #[test]
    fn parse_permutation_tolerates_trailing_prose() {
        // RC4c-2c (GR24 llama.cpp parity): a model may append an explanation AFTER the
        // array. We parse the LEADING array and ignore the trailing bytes (SN-8: still a
        // value boundary at position 0, no mid-string scan).
        assert_eq!(
            parse_permutation("[1,0] because passage 1 is most relevant", 2),
            Some(vec![1, 0])
        );
        assert_eq!(
            parse_permutation("[2,0,1]\n\nExplanation: tectonics is off-topic.", 3),
            Some(vec![2, 0, 1])
        );
        // reasoning preamble stripped THEN a trailing explanation ignored.
        assert_eq!(
            parse_permutation("<|channel>let me rank<channel|>[2,0,1] done", 3),
            Some(vec![2, 0, 1])
        );
        // trailing text after a code fence is already discarded by the fence strip.
        assert_eq!(
            parse_permutation("```json\n[0,1]\n```\nHere you go.", 2),
            Some(vec![0, 1])
        );
        // LEADING prose still fails closed — the array must start the (stripped) body.
        assert_eq!(parse_permutation("The order is [1,0]", 2), None);
    }

    #[test]
    fn parse_permutation_fails_closed_on_non_permutations() {
        // wrong length (too long / too short)
        assert_eq!(parse_permutation("[0,1,2,3]", 3), None);
        assert_eq!(parse_permutation("[0,1]", 3), None);
        // out of range
        assert_eq!(parse_permutation("[0,1,3]", 3), None);
        // duplicate
        assert_eq!(parse_permutation("[0,1,1]", 3), None);
        // negative (a permutation index is never negative)
        assert_eq!(parse_permutation("[-1,0,1]", 3), None);
        // prose / not an array
        assert_eq!(parse_permutation("the best order is 2, 0, 1", 3), None);
        assert_eq!(parse_permutation("{\"order\":[0,1,2]}", 3), None);
        // empty body (an unclosed reasoning tag yields "")
        assert_eq!(parse_permutation("<think>oops", 3), None);
    }

    #[test]
    fn parse_permutation_n_zero_only_accepts_empty_array() {
        assert_eq!(parse_permutation("[]", 0), Some(vec![]));
        assert_eq!(parse_permutation("[0]", 0), None);
    }

    #[test]
    fn extract_answer_unwraps_only_the_exact_answer_arm() {
        // The union answer arm ⇒ unwrapped to its inner text.
        assert_eq!(
            extract_answer(br#"{"answer":"Team shipped the release."}"#).as_ref(),
            b"Team shipped the release."
        );
        // Whitespace-tolerant (Ollama pretty-prints).
        assert_eq!(
            extract_answer(b"{\n  \"answer\": \"hi\"\n}").as_ref(),
            b"hi"
        );
    }

    #[test]
    fn extract_answer_is_a_byte_identical_noop_for_everything_else() {
        // Prose ⇒ verbatim (Cow::Borrowed).
        let prose = b"Just a plain-text answer.";
        assert!(matches!(extract_answer(prose), Cow::Borrowed(_)));
        assert_eq!(extract_answer(prose).as_ref(), prose);
        // A tool_call envelope ⇒ verbatim (never unwrapped — it must still parse as a call).
        let call = br#"{"tool_call":{"name":"slack/read_channel","version":"1","args":{}}}"#;
        assert!(matches!(extract_answer(call), Cow::Borrowed(_)));
        // A `{"answer":…}` with a STRAY field ⇒ verbatim (deny_unknown_fields).
        let stray = br#"{"answer":"x","note":"y"}"#;
        assert!(matches!(extract_answer(stray), Cow::Borrowed(_)));
        // A non-string answer ⇒ verbatim.
        assert!(matches!(
            extract_answer(br#"{"answer":42}"#),
            Cow::Borrowed(_)
        ));
        // Non-JSON / non-object ⇒ verbatim.
        assert!(matches!(extract_answer(b"[1,2,3]"), Cow::Borrowed(_)));
        assert!(matches!(extract_answer(b"not json"), Cow::Borrowed(_)));
    }

    #[test]
    fn extract_answer_arm_still_classifies_as_a_settle() {
        // The union answer arm must be a SETTLE at the parser (Ok(None)), NOT a refusal —
        // this is why no classification change was needed (the answer arm is off-envelope).
        let w = warrant_granting(Some(("slack/read_channel", "1")));
        assert_eq!(parse_tool_call(br#"{"answer":"done"}"#, &w, 4096), Ok(None));
    }
}
