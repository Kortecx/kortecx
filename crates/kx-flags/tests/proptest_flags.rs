// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction. The `pedantic` group
// is also allowed here — tests routinely do things pedantic flags (helper-fn
// definitions after let-bindings, etc.) that would be needless friction.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Property tests for the feature-flag seam.
//!
//! The seam's correctness contract:
//! - **Pure**: identical inputs yield identical outputs; no global state.
//! - **Total**: every input — including arbitrary garbage — returns a `bool`.
//! - **Fail-dark**: no input that is not an explicit truthy value can turn a
//!   default-off flag on. This is the one that matters: a flag that fails OPEN
//!   would ship an unfinished feature to everyone.
//!
//! Properties:
//!
//! 1. `resolve` is total — no (flag, canonical, legacy) triple panics.
//! 2. `resolve` is deterministic — same inputs → same answer.
//! 3. Default-off: every registered flag with no env set resolves `false`.
//! 4. Fail-dark: only an explicit truthy value enables a flag; arbitrary
//!    strings resolve to the default.
//! 5. ENV override: a truthy canonical enables, a falsy canonical disables.
//! 6. Precedence: a recognized canonical always beats the legacy alias.
//! 7. Fall-through: an *un*recognized canonical defers to the legacy alias
//!    rather than deciding.
//! 8. Parsing is trim- and case-insensitive.
//! 9. Registry invariants: default-off, unique names, unique env vars,
//!    `KX_FLAG_` prefix, no alias/canonical collision.

use std::collections::HashSet;

use kx_flags::{parse_bool, resolve, Flag, ENV_PREFIX};
use proptest::prelude::*;

// Strategies

/// Any registered flag.
fn arb_flag() -> impl Strategy<Value = Flag> {
    proptest::sample::select(Flag::ALL)
}

/// The accepted truthy spellings, in arbitrary case with arbitrary padding.
fn arb_truthy() -> impl Strategy<Value = String> {
    (
        proptest::sample::select(vec!["1", "true", "yes", "on"]),
        arb_padding(),
        any::<bool>(),
    )
        .prop_map(|(v, (lead, trail), upper)| {
            let v = if upper {
                v.to_ascii_uppercase()
            } else {
                v.to_string()
            };
            format!("{lead}{v}{trail}")
        })
}

/// The accepted falsy spellings, in arbitrary case with arbitrary padding.
fn arb_falsy() -> impl Strategy<Value = String> {
    (
        proptest::sample::select(vec!["0", "false", "no", "off"]),
        arb_padding(),
        any::<bool>(),
    )
        .prop_map(|(v, (lead, trail), upper)| {
            let v = if upper {
                v.to_ascii_uppercase()
            } else {
                v.to_string()
            };
            format!("{lead}{v}{trail}")
        })
}

/// Whitespace that `parse_bool` must trim.
fn arb_padding() -> impl Strategy<Value = (String, String)> {
    let ws = prop::collection::vec(prop::sample::select(vec![' ', '\t', '\n']), 0..3)
        .prop_map(|v| v.into_iter().collect::<String>());
    (ws.clone(), ws)
}

/// Any string that is NOT an accepted boolean spelling — the "no opinion" space.
fn arb_unrecognized() -> impl Strategy<Value = String> {
    any::<String>().prop_filter("must not be a recognized boolean", |s| {
        parse_bool(Some(s)).is_none()
    })
}

proptest! {
    /// 1 + 2: total and deterministic over arbitrary raw inputs.
    #[test]
    fn resolve_is_total_and_deterministic(
        flag in arb_flag(),
        canonical in any::<Option<String>>(),
        legacy in any::<Option<String>>(),
    ) {
        let a = resolve(&flag, canonical.as_deref(), legacy.as_deref());
        let b = resolve(&flag, canonical.as_deref(), legacy.as_deref());
        prop_assert_eq!(a, b);
    }

    /// 3: unset ⇒ off, for every registered flag.
    #[test]
    fn unset_resolves_to_the_default_which_is_off(flag in arb_flag()) {
        prop_assert!(!resolve(&flag, None, None));
        prop_assert!(!flag.default);
    }

    /// 4: the fail-dark property. No arbitrary string enables a flag.
    #[test]
    fn arbitrary_values_never_enable_a_flag(
        flag in arb_flag(),
        canonical in arb_unrecognized(),
        legacy in arb_unrecognized(),
    ) {
        prop_assert!(!resolve(&flag, Some(&canonical), Some(&legacy)));
    }

    /// 5: an explicit truthy/falsy canonical decides.
    #[test]
    fn truthy_enables_and_falsy_disables(
        flag in arb_flag(),
        on in arb_truthy(),
        off in arb_falsy(),
    ) {
        prop_assert!(resolve(&flag, Some(&on), None));
        prop_assert!(!resolve(&flag, Some(&off), None));
    }

    /// 6: a recognized canonical always wins over the legacy alias.
    #[test]
    fn canonical_beats_legacy(
        flag in arb_flag(),
        canonical in prop_oneof![arb_truthy(), arb_falsy()],
        legacy in prop_oneof![arb_truthy(), arb_falsy()],
    ) {
        let expected = parse_bool(Some(&canonical)).unwrap();
        prop_assert_eq!(resolve(&flag, Some(&canonical), Some(&legacy)), expected);
    }

    /// 7: an unrecognized canonical defers rather than deciding — so a typo in
    /// the new name cannot mask a legacy value that is doing real work.
    #[test]
    fn unrecognized_canonical_falls_through_to_legacy(
        flag in arb_flag(),
        canonical in arb_unrecognized(),
        legacy in prop_oneof![arb_truthy(), arb_falsy()],
    ) {
        prop_assume!(flag.legacy_env.is_some());
        let expected = parse_bool(Some(&legacy)).unwrap();
        prop_assert_eq!(resolve(&flag, Some(&canonical), Some(&legacy)), expected);
    }

    /// 8: parsing ignores surrounding whitespace and case.
    #[test]
    fn parsing_is_trim_and_case_insensitive(on in arb_truthy(), off in arb_falsy()) {
        prop_assert_eq!(parse_bool(Some(&on)), Some(true));
        prop_assert_eq!(parse_bool(Some(&off)), Some(false));
    }
}

/// 9: the registry invariants. Not a proptest — the registry is a fixed slice, so
/// these are exhaustive over it, which is stronger than sampling.
#[test]
fn registry_invariants_hold() {
    let mut names = HashSet::new();
    let mut envs = HashSet::new();
    let mut legacies = HashSet::new();

    for flag in Flag::ALL {
        assert!(!flag.default, "{}: every flag must default OFF", flag.name);

        assert!(
            flag.env.starts_with(ENV_PREFIX),
            "{}: canonical env {} must start with {ENV_PREFIX}",
            flag.name,
            flag.env
        );
        assert_eq!(
            flag.env,
            format!("{ENV_PREFIX}{}", flag.name.to_ascii_uppercase()),
            "{}: canonical env must be {ENV_PREFIX}<NAME>",
            flag.name
        );
        assert_eq!(
            flag.name,
            flag.name.to_ascii_lowercase(),
            "{}: name must be snake_case",
            flag.name
        );

        assert!(
            names.insert(flag.name),
            "duplicate flag name: {}",
            flag.name
        );
        assert!(envs.insert(flag.env), "duplicate env var: {}", flag.env);
        if let Some(legacy) = flag.legacy_env {
            assert!(legacies.insert(legacy), "duplicate legacy alias: {legacy}");
            assert_ne!(
                legacy, flag.env,
                "{}: alias must differ from canonical",
                flag.name
            );
        }
    }

    // A legacy alias must not be some other flag's canonical name, or setting one
    // flag would quietly move another.
    for legacy in &legacies {
        assert!(
            !envs.contains(legacy),
            "legacy alias {legacy} collides with a canonical env var"
        );
    }
}
