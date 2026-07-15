//! Resolution: the pure decision table, plus the thin env read on top of it.
//!
//! The split is deliberate. [`resolve`] takes the raw values rather than reading
//! them, so every branch is testable without `set_var` — which would race under
//! the parallel test runner and leak across tests. [`enabled`] is the only part
//! that touches the process environment, and it does so at point-of-use.

use crate::flag::Flag;

/// The prefix every canonical flag env var carries. Property-tested over
/// [`Flag::ALL`], so the registry cannot drift from it.
pub const ENV_PREFIX: &str = "KX_FLAG_";

/// Parse a flag value: `1`/`true`/`yes`/`on` ⇒ `Some(true)`, `0`/`false`/`no`/`off`
/// ⇒ `Some(false)`, anything else (including an empty or absent value) ⇒ `None`,
/// meaning "no opinion — fall through". Trims, ignores case. Pure + total.
///
/// Returning `None` rather than a bool is what makes precedence expressible: an
/// unrecognized value defers to the next source instead of asserting `false`.
#[must_use]
pub fn parse_bool(raw: Option<&str>) -> Option<bool> {
    match raw?.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Resolve a flag from its raw sources, highest precedence first: the canonical
/// `KX_FLAG_<NAME>` value, then the legacy alias, then the flag's default.
///
/// Pure + total: no input panics, and an unrecognized value at one level falls
/// through to the next rather than silently deciding. `legacy` is ignored when the
/// flag declares no alias, so passing one cannot invent precedence that the
/// registry did not grant.
#[must_use]
pub fn resolve(flag: &Flag, canonical: Option<&str>, legacy: Option<&str>) -> bool {
    // `and` drops the legacy value for a flag that declares no alias.
    let legacy = flag.legacy_env.and(legacy);

    parse_bool(canonical)
        .or_else(|| parse_bool(legacy))
        .unwrap_or(flag.default)
}

/// Is this flag on? Reads the environment at point-of-use — no global cache, so
/// there is no init order to get wrong and nothing to reset between tests.
///
/// The environment is constant for a process's lifetime, so repeated calls within
/// a run agree with each other.
#[must_use]
pub fn enabled(flag: &Flag) -> bool {
    let canonical = std::env::var(flag.env).ok();
    let legacy = flag.legacy_env.and_then(|k| std::env::var(k).ok());

    resolve(flag, canonical.as_deref(), legacy.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_ALIAS: Flag = Flag {
        name: "no_alias",
        env: "KX_FLAG_NO_ALIAS",
        legacy_env: None,
        default: false,
    };

    #[test]
    fn unset_is_off() {
        for flag in Flag::ALL {
            assert!(!resolve(flag, None, None), "{} must default OFF", flag.name);
        }
    }

    #[test]
    fn canonical_env_turns_it_on_and_off() {
        let f = &Flag::SERVE_MEMORY;
        assert!(resolve(f, Some("1"), None));
        assert!(!resolve(f, Some("0"), None));
    }

    #[test]
    fn legacy_alias_still_works() {
        // The whole point of the alias: a knob that shipped under the old name
        // keeps working after it moves onto the seam.
        assert!(resolve(&Flag::SERVE_MEMORY, None, Some("1")));
    }

    #[test]
    fn canonical_beats_legacy() {
        let f = &Flag::SERVE_MEMORY;
        assert!(!resolve(f, Some("0"), Some("1")), "canonical off must win");
        assert!(resolve(f, Some("1"), Some("0")), "canonical on must win");
    }

    #[test]
    fn unrecognized_canonical_falls_through_to_legacy() {
        // "maybe" is not an opinion, so the legacy value still decides — rather
        // than the typo silently pinning the flag off.
        assert!(resolve(&Flag::SERVE_MEMORY, Some("maybe"), Some("1")));
    }

    #[test]
    fn unrecognized_value_never_flips_a_flag_on() {
        for raw in ["", " ", "maybe", "TRUE-ish", "2", "-1", "null", "off!"] {
            assert!(
                !resolve(&Flag::SERVE_MEMORY, Some(raw), None),
                "{raw:?} must not enable a default-off flag"
            );
        }
    }

    #[test]
    fn parsing_trims_and_ignores_case() {
        assert_eq!(parse_bool(Some("  TrUe ")), Some(true));
        assert_eq!(parse_bool(Some("\tOFF\n")), Some(false));
        assert_eq!(parse_bool(Some("YES")), Some(true));
    }

    #[test]
    fn legacy_is_ignored_when_the_flag_declares_no_alias() {
        assert!(!resolve(&NO_ALIAS, None, Some("1")));
    }
}
