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
}
