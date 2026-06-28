//! The [`PriceBook`] — integer micro-USD per-turn / per-tool-call rates and the
//! pure spend fold over the runtime's durable turn/tool counters.

/// Default per-model-turn rate (micro-USD). A notional local-runtime rate — the
/// operator overrides it via [`ENV_PER_TURN_MICRO_USD`] (or the serve config). The
/// default is non-zero so the guardrail prices something out of the box, but the
/// `cost_ceiling` axis defaults to `0` (= unset / OFF), so nothing is enforced
/// until an operator opts in with a positive ceiling.
pub const DEFAULT_PER_TURN_MICRO_USD: u64 = 1_000;

/// Default per-tool-call rate (micro-USD). See [`DEFAULT_PER_TURN_MICRO_USD`].
pub const DEFAULT_PER_TOOL_CALL_MICRO_USD: u64 = 500;

/// Env var the host reads to override the per-turn rate (integer micro-USD).
pub const ENV_PER_TURN_MICRO_USD: &str = "KX_PRICING_PER_TURN_MICRO_USD";

/// Env var the host reads to override the per-tool-call rate (integer micro-USD).
pub const ENV_PER_TOOL_CALL_MICRO_USD: &str = "KX_PRICING_PER_TOOL_CALL_MICRO_USD";

/// A local, operator-priced cost model: integer micro-USD rates for a model turn
/// and a tool call. The spend ESTIMATE is a pure, total, saturating fold over the
/// runtime's durable turn/tool counters (see the crate docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriceBook {
    /// Micro-USD charged per model turn.
    pub per_turn_micro_usd: u64,
    /// Micro-USD charged per tool call.
    pub per_tool_call_micro_usd: u64,
}

impl Default for PriceBook {
    fn default() -> Self {
        Self {
            per_turn_micro_usd: DEFAULT_PER_TURN_MICRO_USD,
            per_tool_call_micro_usd: DEFAULT_PER_TOOL_CALL_MICRO_USD,
        }
    }
}

impl PriceBook {
    /// Construct an explicit price-book.
    #[must_use]
    pub const fn new(per_turn_micro_usd: u64, per_tool_call_micro_usd: u64) -> Self {
        Self {
            per_turn_micro_usd,
            per_tool_call_micro_usd,
        }
    }

    /// The spend ESTIMATE (micro-USD) for a run that has used `turns` model turns
    /// and `tool_calls` tool calls. A pure, total, **saturating** fold — a runaway
    /// count can never wrap the ceiling (it pins to `u64::MAX`, which fails closed
    /// against any finite ceiling).
    #[must_use]
    pub const fn estimate_spend(&self, turns: u64, tool_calls: u64) -> u64 {
        let turn_cost = turns.saturating_mul(self.per_turn_micro_usd);
        let tool_cost = tool_calls.saturating_mul(self.per_tool_call_micro_usd);
        turn_cost.saturating_add(tool_cost)
    }

    /// Resolve a price-book from the environment, falling back to `self` for any
    /// var that is absent or unparseable (a malformed override never panics and
    /// never silently zeroes a rate). A host-side convenience; reads env at
    /// runtime only (no `build.rs`, no I1.c byte-determinism impact).
    #[must_use]
    pub fn with_env_overrides(self) -> Self {
        Self {
            per_turn_micro_usd: parse_env_u64(ENV_PER_TURN_MICRO_USD, self.per_turn_micro_usd),
            per_tool_call_micro_usd: parse_env_u64(
                ENV_PER_TOOL_CALL_MICRO_USD,
                self.per_tool_call_micro_usd,
            ),
        }
    }
}

/// Read `name` from the environment as a `u64`, falling back to `default` when the
/// var is absent or does not parse (fail-safe: never zero a rate by accident).
fn parse_env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_is_turns_times_per_turn_plus_tools_times_per_tool() {
        let pb = PriceBook::new(1_000, 500);
        assert_eq!(pb.estimate_spend(0, 0), 0);
        assert_eq!(pb.estimate_spend(3, 0), 3_000);
        assert_eq!(pb.estimate_spend(0, 4), 2_000);
        assert_eq!(pb.estimate_spend(3, 4), 3_000 + 2_000);
    }

    #[test]
    fn estimate_saturates_and_never_wraps() {
        // A runaway count pins to u64::MAX (fails closed against any finite ceiling)
        // rather than wrapping to a small value.
        let pb = PriceBook::new(u64::MAX, u64::MAX);
        assert_eq!(pb.estimate_spend(u64::MAX, u64::MAX), u64::MAX);
        assert_eq!(pb.estimate_spend(2, 0), u64::MAX);
    }

    #[test]
    fn default_rates_are_nonzero() {
        let pb = PriceBook::default();
        assert_eq!(pb.per_turn_micro_usd, DEFAULT_PER_TURN_MICRO_USD);
        assert_eq!(pb.per_tool_call_micro_usd, DEFAULT_PER_TOOL_CALL_MICRO_USD);
        assert!(pb.estimate_spend(1, 1) > 0);
    }

    #[test]
    fn env_overrides_fall_back_on_absent_or_malformed() {
        // Absent vars ⇒ the base rates are kept (this test does not set the vars).
        let base = PriceBook::new(7, 9);
        let resolved = base.with_env_overrides();
        // In a clean env the override is a no-op; the test asserts the fall-back
        // contract rather than mutating process-global env (which would race other
        // tests). A malformed/absent var keeps the base rate.
        assert_eq!(resolved.per_turn_micro_usd, 7);
        assert_eq!(resolved.per_tool_call_micro_usd, 9);
    }
}
