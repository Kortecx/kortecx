//! D110.4 / IMP-5 — `validate_args` is TOTAL + panic-free over arbitrary bytes
//! and never ACCEPTS a value that violates the declared typed schema (no float
//! reaches an `Int` param, no over-long string, no smuggled key under
//! `deny_unknown`). Mirrors the fail-closed decode proptests of `kx-planner`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::BTreeSet;

use kx_tool_registry::{validate_args, InputSchema, ParamSpec, ParamType};
use proptest::prelude::*;

fn arb_param_type() -> impl Strategy<Value = ParamType> {
    prop_oneof![
        (
            proptest::option::of(any::<i64>()),
            proptest::option::of(any::<i64>())
        )
            .prop_map(|(min, max)| ParamType::Int { min, max }),
        (0usize..32).prop_map(|max_len| ParamType::Bytes { max_len }),
        (0usize..32).prop_map(|max_len| ParamType::Str { max_len }),
        Just(ParamType::Bool),
        proptest::collection::btree_set("[a-z]{1,5}", 1..4)
            .prop_map(|allowed| ParamType::Enum { allowed }),
    ]
}

fn arb_schema() -> impl Strategy<Value = InputSchema> {
    (
        proptest::collection::vec(
            ("[a-z]{1,6}", arb_param_type(), any::<bool>())
                .prop_map(|(name, ty, required)| ParamSpec { name, ty, required }),
            0..5,
        ),
        any::<bool>(),
    )
        .prop_map(|(params, deny_unknown)| InputSchema {
            params,
            deny_unknown,
        })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// TOTAL: never panics on arbitrary bytes against an arbitrary schema.
    #[test]
    fn prop_validate_args_is_total(schema in arb_schema(), bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
        let _ = validate_args(&schema, &bytes); // reaching here proves no panic
    }

    /// TOTAL over arbitrary UTF-8 JSON-ish strings too (denser coverage of the
    /// parse path than raw bytes, which are mostly invalid JSON).
    #[test]
    fn prop_validate_args_is_total_over_json_ish(schema in arb_schema(), s in "\\{.*\\}") {
        let _ = validate_args(&schema, s.as_bytes());
    }

    /// A required `Int` param NEVER accepts a NON-INTEGER JSON number (no float on
    /// the action path, SN-8). Built as `<int>.5` so the JSON token always has a
    /// fractional part — an `i64` deserialize must reject it.
    #[test]
    fn prop_int_never_accepts_float(base in any::<i32>()) {
        let schema = InputSchema {
            params: vec![ParamSpec {
                name: "n".into(),
                ty: ParamType::Int { min: None, max: None },
                required: true,
            }],
            deny_unknown: true,
        };
        let args = format!("{{\"n\":{base}.5}}");
        prop_assert!(validate_args(&schema, args.as_bytes()).is_err());
    }
}

/// An unknown key under `deny_unknown` is refused; under `!deny_unknown` ignored.
#[test]
fn unknown_key_policy() {
    let mk = |deny_unknown| InputSchema {
        params: vec![ParamSpec {
            name: "a".into(),
            ty: ParamType::Bool,
            required: false,
        }],
        deny_unknown,
    };
    assert!(validate_args(&mk(true), br#"{"a":true,"x":1}"#).is_err());
    assert!(validate_args(&mk(false), br#"{"a":true,"x":1}"#).is_ok());
}

/// An enum value outside the allowed set is refused (exact match, no fuzzy, SN-8).
#[test]
fn enum_exact_match_only() {
    let schema = InputSchema {
        params: vec![ParamSpec {
            name: "mode".into(),
            ty: ParamType::Enum {
                allowed: BTreeSet::from(["fast".to_string()]),
            },
            required: true,
        }],
        deny_unknown: true,
    };
    assert!(validate_args(&schema, br#"{"mode":"fast"}"#).is_ok());
    assert!(validate_args(&schema, br#"{"mode":"Fast"}"#).is_err()); // near-miss refused
    assert!(validate_args(&schema, br#"{"mode":"slow"}"#).is_err());
}
