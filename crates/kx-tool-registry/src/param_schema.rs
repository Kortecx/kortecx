//! [`InputSchema`] — a tool's declared, typed parameter contract, and
//! [`validate_args`], the fail-closed validator for a model's proposed tool-call
//! arguments (D110.4; meets D83's `free_param_schema`).
//!
//! Adopts the MCP `inputSchema` idea but as a **closed, integer-/bytes-typed**
//! schema — there is **no `Float` variant**, so no float ever reaches the action
//! path (SN-8). A model-proposed argument bag is untrusted JSON; it is validated
//! against the tool's declared schema **before** dispatch, mirroring the
//! fail-closed, total, panic-free decode discipline of `kx_planner::decode` and
//! `kx_mcp::decode` (size-cap is already applied upstream by
//! `kx_model_harness::toolcall::parse_tool_call`, IMP-16). A tool with no schema
//! (`input_schema: None`) is dispatched exactly as before (no validation).

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

/// A tool parameter's declared type. A CLOSED set with **no float** (SN-8 / D83):
/// integers are exact, bytes/strings are length-bounded, enums are exact-match.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ParamType {
    /// A signed 64-bit integer, with optional inclusive `[min, max]` bounds.
    /// Decoding rejects any non-integer JSON token (a float like `1.5` fails).
    Int {
        /// Inclusive lower bound, if any.
        min: Option<i64>,
        /// Inclusive upper bound, if any.
        max: Option<i64>,
    },
    /// A JSON string treated as opaque bytes, bounded to `max_len` UTF-8 bytes.
    Bytes {
        /// Maximum length in bytes.
        max_len: usize,
    },
    /// A UTF-8 string, bounded to `max_len` bytes.
    Str {
        /// Maximum length in bytes.
        max_len: usize,
    },
    /// A boolean.
    Bool,
    /// An exact-match against a fixed set of allowed string values.
    Enum {
        /// The permitted values (exact equality; no fuzzy match, SN-8).
        allowed: BTreeSet<String>,
    },
}

/// A single declared parameter: its name, type, and whether it is required.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamSpec {
    /// The argument key the model must use.
    pub name: String,
    /// The declared type the argument's value must satisfy.
    pub ty: ParamType,
    /// Whether the argument must be present.
    pub required: bool,
}

/// A tool's declared typed parameter schema (the MCP `inputSchema` analogue).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputSchema {
    /// The declared parameters (canonical order — part of the tool's identity).
    pub params: Vec<ParamSpec>,
    /// If `true`, an argument key not in `params` is refused (fail-closed against
    /// smuggled fields). If `false`, unknown keys are ignored.
    pub deny_unknown: bool,
}

/// Why a model-proposed argument bag failed [`validate_args`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    /// The args were not a JSON object.
    NotAnObject,
    /// A required parameter was absent.
    MissingRequired {
        /// The missing parameter's name.
        name: String,
    },
    /// An argument key is not a declared parameter and `deny_unknown` is set.
    UnknownParam {
        /// The offending key.
        name: String,
    },
    /// A parameter's value did not match its declared type.
    TypeMismatch {
        /// The parameter's name.
        name: String,
        /// The declared type, for diagnostics.
        expected: &'static str,
    },
    /// An integer parameter's value was outside its declared `[min, max]`.
    OutOfRange {
        /// The parameter's name.
        name: String,
    },
    /// A bytes/string parameter exceeded its declared `max_len`.
    TooLong {
        /// The parameter's name.
        name: String,
        /// The declared maximum.
        max: usize,
    },
    /// An enum parameter's value was not in the allowed set.
    NotAllowed {
        /// The parameter's name.
        name: String,
    },
    /// The args bytes were not well-formed JSON.
    Malformed {
        /// A short, non-secret diagnostic.
        diagnostic: String,
    },
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaError::NotAnObject => write!(f, "args are not a JSON object"),
            SchemaError::MissingRequired { name } => {
                write!(f, "missing required parameter `{name}`")
            }
            SchemaError::UnknownParam { name } => {
                write!(f, "unknown parameter `{name}` (deny_unknown)")
            }
            SchemaError::TypeMismatch { name, expected } => {
                write!(f, "parameter `{name}` is not a {expected}")
            }
            SchemaError::OutOfRange { name } => write!(f, "parameter `{name}` is out of range"),
            SchemaError::TooLong { name, max } => {
                write!(f, "parameter `{name}` exceeds max length {max}")
            }
            SchemaError::NotAllowed { name } => {
                write!(f, "parameter `{name}` is not an allowed value")
            }
            SchemaError::Malformed { diagnostic } => write!(f, "malformed args: {diagnostic}"),
        }
    }
}

impl std::error::Error for SchemaError {}

/// Validate a model's proposed tool-call `args_bytes` against `schema`,
/// **fail-closed**. Total + panic-free over arbitrary bytes: the args are parsed
/// as a SHALLOW one-level map of raw values (never a recursive dynamic `Value`,
/// so no float / NaN / unbounded-recursion ever reaches the action path), then
/// each declared parameter is checked by deserializing its raw value into the
/// EXACT Rust type for its [`ParamType`].
///
/// # Errors
///
/// [`SchemaError`] on any structural or type mismatch — the dispatch is then
/// refused before any effect fires.
pub fn validate_args(schema: &InputSchema, args_bytes: &[u8]) -> Result<(), SchemaError> {
    // PR-3 (A3c): tolerate the single most common, UNAMBIGUOUS model JSON
    // malformation — a trailing comma — by normalizing FIRST, so the same bytes
    // that validate are the bytes that fire (the coordinator re-derives the
    // normalized form for `WorkItem.tool_args` — `normalize_lenient_args`).
    // This relaxes only the arg SYNTAX surface, never the authority gate
    // (name/grant resolution stays exact — SN-8).
    let normalized = normalize_lenient_args(args_bytes);
    let args_bytes: &[u8] = normalized.as_ref();
    // Empty args == `{}` (the no-arguments case), mirroring the MCP capability.
    let map: BTreeMap<String, &RawValue> = if args_bytes.is_empty() {
        BTreeMap::new()
    } else {
        serde_json::from_slice(args_bytes).map_err(|e| {
            // A non-object (array, scalar) or malformed body: classify NotAnObject
            // for the common "not an object" case, else Malformed.
            if e.is_data() {
                SchemaError::NotAnObject
            } else {
                SchemaError::Malformed {
                    diagnostic: e.to_string(),
                }
            }
        })?
    };

    if schema.deny_unknown {
        let declared: BTreeSet<&str> = schema.params.iter().map(|p| p.name.as_str()).collect();
        for key in map.keys() {
            if !declared.contains(key.as_str()) {
                return Err(SchemaError::UnknownParam { name: key.clone() });
            }
        }
    }

    for spec in &schema.params {
        match map.get(&spec.name) {
            None => {
                if spec.required {
                    return Err(SchemaError::MissingRequired {
                        name: spec.name.clone(),
                    });
                }
            }
            Some(raw) => check_value(&spec.name, &spec.ty, raw)?,
        }
    }
    Ok(())
}

/// PR-3 (A3c): normalize a model's proposed args bytes by repairing the JSON5-style
/// malformations a real model emits in a tool call — the **strict-JSON subset a
/// live Gemma-4-12B actually produces** (witnessed across re-runs):
///
/// 1. a **trailing comma** before a closing `}` / `]` (`{"text":"hi",}`);
/// 2. an **unquoted object key** (`{text:"hi"}` → `{"text":"hi"}`); and
/// 3. a **single-quoted string** (`{text:'hi'}` → `{"text":"hi"}`, escaping any
///    interior `"`), the value form Gemma emits (*"expected value at column 10"*).
///
/// PURE + total + panic-free + deterministic + idempotent (`f(f(x)) == f(x)`) over
/// ARBITRARY bytes (proptest-pinned). A context-aware single pass tracks
/// double-quoted-string + single-quoted-string + escape state (a comma/identifier/
/// quote INSIDE a string is never touched) and a container stack so a key is quoted
/// ONLY at an object-member start. Returns `Borrowed` when nothing changes.
///
/// SN-8: this relaxes ARG SYNTAX only — a key/value is preserved BYTE-FOR-BYTE
/// (only its delimiters change: `text`→`"text"`, `'hi'`→`"hi"`), so the parameter
/// NAME the schema matches stays EXACT; it never fuzzy-matches a name, coerces a
/// value type, or widens a grant. ACCEPT-side best effort: if the repair still does
/// not parse, `validate_args` refuses fail-closed (an un-repairable bag is never
/// fired). Comments and unquoted VALUES stay fail-closed (deliberately out of scope).
#[must_use]
#[allow(clippy::too_many_lines)] // one cohesive JSON5-repair state machine
pub fn normalize_lenient_args(args: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    // Fast path: scan once; only allocate if a repair is actually needed.
    if !needs_lenient_repair(args) {
        return std::borrow::Cow::Borrowed(args);
    }
    let mut out: Vec<u8> = Vec::with_capacity(args.len() + 8);
    let mut escaped = false;
    let mut stack: Vec<u8> = Vec::new();
    let mut expect_key = false; // true at an object-member start
    let mut i = 0usize;
    while i < args.len() {
        let b = args[i];
        match b {
            b'"' => {
                // A double-quoted string: copy verbatim through its close.
                expect_key = false;
                out.push(b);
                i += 1;
                while i < args.len() {
                    let c = args[i];
                    out.push(c);
                    i += 1;
                    if escaped {
                        escaped = false;
                    } else if c == b'\\' {
                        escaped = true;
                    } else if c == b'"' {
                        break;
                    }
                }
            }
            b'\'' => {
                // A SINGLE-quoted string → re-emit as double-quoted, escaping any
                // interior `"` and unescaping `\'`. Copies the content byte-for-byte
                // otherwise (the value is preserved exactly).
                expect_key = false;
                out.push(b'"');
                i += 1;
                while i < args.len() {
                    let c = args[i];
                    if escaped {
                        // `\'` → `'`; every other escape passes through verbatim.
                        if c == b'\'' {
                            out.push(b'\'');
                        } else {
                            out.push(b'\\');
                            out.push(c);
                        }
                        escaped = false;
                        i += 1;
                    } else if c == b'\\' {
                        escaped = true;
                        i += 1;
                    } else if c == b'\'' {
                        out.push(b'"'); // close
                        i += 1;
                        break;
                    } else if c == b'"' {
                        out.push(b'\\'); // escape an interior double-quote
                        out.push(b'"');
                        i += 1;
                    } else {
                        out.push(c);
                        i += 1;
                    }
                }
            }
            b'{' => {
                stack.push(b'{');
                expect_key = true;
                out.push(b);
                i += 1;
            }
            b'[' => {
                stack.push(b'[');
                expect_key = false;
                out.push(b);
                i += 1;
            }
            b'}' | b']' => {
                stack.pop();
                expect_key = false;
                out.push(b);
                i += 1;
            }
            b',' => {
                // Trailing comma: drop it if the next significant byte closes a
                // container. The lookahead skips whitespace AND further commas, so a
                // RUN of trailing commas (`,,}`) collapses in ONE pass — else the first
                // comma would survive until a second pass dropped the rest, then re-drop
                // it (the idempotency proptest's other failing class).
                let mut j = i + 1;
                while j < args.len() && (args[j].is_ascii_whitespace() || args[j] == b',') {
                    j += 1;
                }
                if j < args.len() && (args[j] == b'}' || args[j] == b']') {
                    i += 1; // skip
                } else {
                    out.push(b);
                    expect_key = stack.last() == Some(&b'{');
                    i += 1;
                }
            }
            b':' => {
                expect_key = false;
                out.push(b);
                i += 1;
            }
            c if c.is_ascii_whitespace() => {
                out.push(b);
                i += 1;
            }
            _ => {
                if expect_key {
                    // An UNQUOTED key: copy the identifier wrapped in quotes. Scan to
                    // the next delimiter (total: the first byte is a non-delimiter, so
                    // this advances ≥ 1). A `\` is NOT a scan delimiter, so the key can
                    // contain a backslash — it MUST be escaped (`\` → `\\`) inside the
                    // emitted string, else a key ending in `\` would make the closing
                    // `"` an escaped quote, leaving the string unterminated (so a
                    // second `normalize_lenient_args` pass would re-parse it differently
                    // — the idempotency proptest's failing class). A `"` cannot appear
                    // in the span (it is a scan delimiter), so `\` is the only escape.
                    let start = i;
                    while i < args.len() {
                        let cc = args[i];
                        if cc == b':' || cc == b'"' || cc == b'\'' || cc.is_ascii_whitespace() {
                            break;
                        }
                        i += 1;
                    }
                    out.push(b'"');
                    for &kc in &args[start..i] {
                        if kc == b'\\' {
                            out.push(b'\\');
                        }
                        out.push(kc);
                    }
                    out.push(b'"');
                    expect_key = false;
                } else {
                    out.push(b);
                    i += 1;
                }
            }
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Cheap pre-scan: does `args` contain a trailing comma, an unquoted object key, OR
/// a single-quoted string? Mirrors [`normalize_lenient_args`]'s state machine but
/// only *detects* (no allocation), so a well-formed bag returns `Borrowed`.
fn needs_lenient_repair(args: &[u8]) -> bool {
    let mut escaped = false;
    let mut stack: Vec<u8> = Vec::new();
    let mut expect_key = false;
    let mut i = 0usize;
    while i < args.len() {
        let b = args[i];
        match b {
            b'\'' => return true, // a single-quoted string (JSON has none)
            b'"' => {
                // Skip a double-quoted string verbatim.
                expect_key = false;
                i += 1;
                while i < args.len() {
                    let c = args[i];
                    i += 1;
                    if escaped {
                        escaped = false;
                    } else if c == b'\\' {
                        escaped = true;
                    } else if c == b'"' {
                        break;
                    }
                }
            }
            b'{' => {
                stack.push(b'{');
                expect_key = true;
                i += 1;
            }
            b'[' => {
                stack.push(b'[');
                expect_key = false;
                i += 1;
            }
            b'}' | b']' => {
                stack.pop();
                expect_key = false;
                i += 1;
            }
            b',' => {
                let mut j = i + 1;
                while j < args.len() && args[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < args.len() && (args[j] == b'}' || args[j] == b']') {
                    return true; // trailing comma
                }
                expect_key = stack.last() == Some(&b'{');
                i += 1;
            }
            b':' => {
                expect_key = false;
                i += 1;
            }
            c if c.is_ascii_whitespace() => i += 1,
            _ => {
                if expect_key {
                    return true; // unquoted key
                }
                i += 1;
            }
        }
    }
    false
}

/// Check one raw JSON value against a declared [`ParamType`] by deserializing into
/// the exact Rust type — never `serde_json::Value` (no float/recursion path).
fn check_value(name: &str, ty: &ParamType, raw: &RawValue) -> Result<(), SchemaError> {
    let s = raw.get();
    match ty {
        ParamType::Int { min, max } => {
            // `i64` deserialize rejects float tokens (`1.5`), strings, etc.
            let v: i64 = serde_json::from_str(s).map_err(|_| SchemaError::TypeMismatch {
                name: name.to_string(),
                expected: "integer",
            })?;
            if min.is_some_and(|lo| v < lo) || max.is_some_and(|hi| v > hi) {
                return Err(SchemaError::OutOfRange {
                    name: name.to_string(),
                });
            }
            Ok(())
        }
        ParamType::Bytes { max_len } | ParamType::Str { max_len } => {
            let v: String = serde_json::from_str(s).map_err(|_| SchemaError::TypeMismatch {
                name: name.to_string(),
                expected: "string",
            })?;
            if v.len() > *max_len {
                return Err(SchemaError::TooLong {
                    name: name.to_string(),
                    max: *max_len,
                });
            }
            Ok(())
        }
        ParamType::Bool => {
            let _: bool = serde_json::from_str(s).map_err(|_| SchemaError::TypeMismatch {
                name: name.to_string(),
                expected: "bool",
            })?;
            Ok(())
        }
        ParamType::Enum { allowed } => {
            let v: String = serde_json::from_str(s).map_err(|_| SchemaError::TypeMismatch {
                name: name.to_string(),
                expected: "string",
            })?;
            if allowed.contains(&v) {
                Ok(())
            } else {
                Err(SchemaError::NotAllowed {
                    name: name.to_string(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> InputSchema {
        InputSchema {
            params: vec![
                ParamSpec {
                    name: "count".into(),
                    ty: ParamType::Int {
                        min: Some(0),
                        max: Some(100),
                    },
                    required: true,
                },
                ParamSpec {
                    name: "label".into(),
                    ty: ParamType::Str { max_len: 8 },
                    required: false,
                },
                ParamSpec {
                    name: "mode".into(),
                    ty: ParamType::Enum {
                        allowed: BTreeSet::from(["fast".to_string(), "slow".to_string()]),
                    },
                    required: false,
                },
            ],
            deny_unknown: true,
        }
    }

    #[test]
    fn accepts_valid_args() {
        assert!(validate_args(&schema(), br#"{"count":5,"label":"hi","mode":"fast"}"#).is_ok());
    }

    #[test]
    fn required_missing_is_refused() {
        assert_eq!(
            validate_args(&schema(), br#"{"label":"hi"}"#),
            Err(SchemaError::MissingRequired {
                name: "count".into()
            })
        );
    }

    #[test]
    fn float_for_int_is_refused() {
        assert_eq!(
            validate_args(&schema(), br#"{"count":1.5}"#),
            Err(SchemaError::TypeMismatch {
                name: "count".into(),
                expected: "integer"
            })
        );
    }

    #[test]
    fn out_of_range_int_is_refused() {
        assert!(matches!(
            validate_args(&schema(), br#"{"count":999}"#),
            Err(SchemaError::OutOfRange { .. })
        ));
    }

    #[test]
    fn over_long_string_is_refused() {
        assert!(matches!(
            validate_args(&schema(), br#"{"count":1,"label":"way-too-long"}"#),
            Err(SchemaError::TooLong { .. })
        ));
    }

    #[test]
    fn unknown_key_is_refused_when_deny_unknown() {
        assert!(matches!(
            validate_args(&schema(), br#"{"count":1,"smuggled":7}"#),
            Err(SchemaError::UnknownParam { .. })
        ));
    }

    #[test]
    fn enum_outside_set_is_refused() {
        assert!(matches!(
            validate_args(&schema(), br#"{"count":1,"mode":"turbo"}"#),
            Err(SchemaError::NotAllowed { .. })
        ));
    }

    #[test]
    fn non_object_is_refused() {
        assert_eq!(
            validate_args(&schema(), b"[1,2,3]"),
            Err(SchemaError::NotAnObject)
        );
    }

    #[test]
    fn empty_args_with_required_is_refused() {
        assert!(matches!(
            validate_args(&schema(), b""),
            Err(SchemaError::MissingRequired { .. })
        ));
    }

    // -----------------------------------------------------------------------
    // PR-3 (A3c) — conservative JSON-malformation tolerance (trailing commas).
    // -----------------------------------------------------------------------

    #[test]
    fn accepts_a_trailing_comma_in_an_object() {
        // The model emits a trailing comma after the last key — now validates.
        assert!(validate_args(&schema(), br#"{"count": 5,}"#).is_ok());
        assert!(validate_args(&schema(), br#"{"count": 5, "label": "ok",}"#).is_ok());
    }

    #[test]
    fn trailing_comma_inside_a_string_is_not_stripped() {
        // A comma-then-brace INSIDE a string value must survive verbatim (it is
        // not a trailing comma). The label `",}"` is 2 bytes ≤ max_len 8 → valid.
        assert_eq!(
            normalize_lenient_args(br#"{"count":1,"label":",}"}"#).as_ref(),
            br#"{"count":1,"label":",}"}"#
        );
        assert!(validate_args(&schema(), br#"{"count":1,"label":",}"}"#).is_ok());
    }

    #[test]
    fn quotes_an_unquoted_object_key_the_gemma_malformation() {
        // The live Gemma-4 witness: the model emits `{text: "pong"}` (unquoted key).
        // The key is quoted BYTE-FOR-BYTE (name preserved exactly, SN-8) so it fires.
        assert_eq!(
            normalize_lenient_args(br"{count: 5}").as_ref(),
            br#"{"count": 5}"#
        );
        assert!(validate_args(&schema(), br"{count: 5}").is_ok());
        // A mixed bag (one quoted, one unquoted key + a trailing comma) repairs whole.
        assert_eq!(
            normalize_lenient_args(br#"{count: 5, "label":"hi",}"#).as_ref(),
            br#"{"count": 5, "label":"hi"}"#
        );
        // An unquoted key INSIDE a string value is NOT touched.
        assert_eq!(
            normalize_lenient_args(br#"{"label":"a:b c"}"#).as_ref(),
            br#"{"label":"a:b c"}"#
        );
        // Array elements are NEVER treated as keys (the `,` after `1` is array-level).
        assert_eq!(
            normalize_lenient_args(br#"{"xs":[1,2,3]}"#).as_ref(),
            br#"{"xs":[1,2,3]}"#
        );
    }

    #[test]
    fn normalize_is_idempotent_and_pure() {
        let dirty = br"{a:[1,2,],b:{c:3,},}";
        let once = normalize_lenient_args(dirty).into_owned();
        let twice = normalize_lenient_args(&once).into_owned();
        assert_eq!(once, twice, "idempotent: f(f(x)) == f(x)");
        // The cleaned bytes: keys quoted, trailing commas dropped.
        assert_eq!(once, br#"{"a":[1,2],"b":{"c":3}}"#);
        // Pure: a clean bag is returned BORROWED (zero allocation).
        assert!(matches!(
            normalize_lenient_args(br#"{"count":5}"#),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn repairs_single_quoted_strings_the_gemma_value_malformation() {
        // The live Gemma-4 witness, post unquoted-key fix: the model emits
        // `{text: 'pong'}` (single-quoted value). The string is preserved
        // byte-for-byte, only its delimiters change (SN-8).
        assert_eq!(
            normalize_lenient_args(br"{'label': 'hi'}").as_ref(),
            br#"{"label": "hi"}"#
        );
        // The full Gemma shape: unquoted key + single-quoted value (with the
        // required `count` present so the whole bag validates end-to-end).
        assert_eq!(
            normalize_lenient_args(br"{label: 'hi'}").as_ref(),
            br#"{"label": "hi"}"#
        );
        assert!(validate_args(&schema(), br"{count: 5, label: 'hi'}").is_ok());
        // An interior double-quote inside a single-quoted string is escaped.
        assert_eq!(
            normalize_lenient_args(br#"{'label': 'a"b'}"#).as_ref(),
            br#"{"label": "a\"b"}"#.as_slice()
        );
        // A single quote INSIDE a double-quoted string is left alone.
        assert_eq!(
            normalize_lenient_args(br#"{"label":"it's"}"#).as_ref(),
            br#"{"label":"it's"}"#
        );
    }

    #[test]
    fn normalize_does_not_invent_validity_for_out_of_scope_malformations() {
        // SN-8 / narrow-scope: an unquoted VALUE (not a string) stays fail-closed
        // (genuinely ambiguous — `five` could be a typo'd keyword, not a string).
        assert!(validate_args(&schema(), br"{count: five}").is_err());
    }

    proptest::proptest! {
        /// Totality + panic-freedom + IDEMPOTENCE over ARBITRARY bytes — the
        /// security-gate discipline. The normalizer never panics; a second pass is
        /// a fixed point (so the repaired bytes are stable across re-derivation /
        /// replay). Growth is bounded (≤ 2 bytes per unquoted key) — quoting adds
        /// the missing quotes — so a tight size bound is intentionally not asserted.
        #[test]
        fn normalize_is_total_and_idempotent(bytes: Vec<u8>) {
            let out = normalize_lenient_args(&bytes).into_owned();
            let again = normalize_lenient_args(&out).into_owned();
            proptest::prop_assert_eq!(again, out);
        }

        /// validate_args is total + panic-free over arbitrary bytes + arbitrary
        /// (well-formed) schema usage — it only ever returns Ok/Err, never panics.
        #[test]
        fn validate_args_is_total(bytes: Vec<u8>) {
            let _ = validate_args(&schema(), &bytes); // must not panic
        }
    }
}
