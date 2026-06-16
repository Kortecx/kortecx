//! Prometheus text exposition rendering — deterministic, allocation-cheap, and
//! dependency-free (the format is line-based; pulling a metrics crate would only
//! add a tonic/version surface for what is a few `writeln!`s).
//!
//! Contract: every family emits exactly one `# HELP` + one `# TYPE` line followed
//! by its sample line(s). A scrape renders a snapshot of [`MetricsState`] plus an
//! optional [`LatencySummary`] the host supplies from its telemetry exhaust — so
//! the durable-fact COUNTERS and the operational-window LATENCY gauges share one
//! formatter (a single source of format truth across the metrics surface).

use std::fmt::Write as _;

use crate::fold::{MetricsState, FAILURE_REASON_LABELS};

/// Build metadata surfaced as `kortecx_build_info{version="…"} 1`.
#[derive(Debug, Clone, Copy)]
pub struct BuildInfo {
    /// The gateway crate version (e.g. `CARGO_PKG_VERSION`).
    pub version: &'static str,
}

/// Recent-window latency + token totals, supplied by the host from its telemetry
/// exhaust (off the durable-fact path — the same numbers the Monitoring UI shows).
/// `None` when no telemetry seam is wired (an FFI-free serve), in which case the
/// duration block is honestly omitted rather than rendered as zero.
#[derive(Debug, Clone, Copy)]
pub struct LatencySummary {
    /// How many recent model Motes the percentiles cover.
    pub window: u64,
    /// Nearest-rank p50 of per-Mote wall-clock milliseconds over the window.
    pub p50_ms: u64,
    /// Nearest-rank p95 of per-Mote wall-clock milliseconds over the window.
    pub p95_ms: u64,
    /// Summed `output_tokens` over the window.
    pub output_tokens: u64,
}

/// A minimal Prometheus text-format writer.
struct PromWriter {
    buf: String,
}

impl PromWriter {
    fn new() -> Self {
        Self {
            // Pre-size for the ~25 lines a typical render emits.
            buf: String::with_capacity(2048),
        }
    }

    /// Emit the `# HELP` + `# TYPE` header for a metric family (once per family).
    fn family(&mut self, name: &str, kind: &str, help: &str) {
        // Infallible: writing to a String never errors.
        let _ = writeln!(self.buf, "# HELP {name} {help}");
        let _ = writeln!(self.buf, "# TYPE {name} {kind}");
    }

    /// Emit one sample line `name{labels} value`.
    fn sample(&mut self, name: &str, labels: &[(&str, &str)], value: u64) {
        self.buf.push_str(name);
        if !labels.is_empty() {
            self.buf.push('{');
            for (i, (k, v)) in labels.iter().enumerate() {
                if i > 0 {
                    self.buf.push(',');
                }
                let _ = write!(self.buf, "{k}=\"");
                escape_label_value(&mut self.buf, v);
                self.buf.push('"');
            }
            self.buf.push('}');
        }
        let _ = writeln!(self.buf, " {value}");
    }

    /// A single-sample counter family.
    fn counter(&mut self, name: &str, help: &str, value: u64) {
        self.family(name, "counter", help);
        self.sample(name, &[], value);
    }

    /// A single-sample gauge family.
    fn gauge(&mut self, name: &str, help: &str, value: u64) {
        self.family(name, "gauge", help);
        self.sample(name, &[], value);
    }

    fn finish(self) -> String {
        self.buf
    }
}

/// Escape a Prometheus label value (`\` → `\\`, `"` → `\"`, newline → `\n`).
fn escape_label_value(buf: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '\\' => buf.push_str(r"\\"),
            '"' => buf.push_str("\\\""),
            '\n' => buf.push_str(r"\n"),
            c => buf.push(c),
        }
    }
}

/// Render the full `/metrics` body for a `MetricsState` snapshot plus optional
/// recent-window latency. Pure + deterministic: the same inputs always yield the
/// same bytes (unit-tested as a golden string).
#[must_use]
pub fn render(state: &MetricsState, build: &BuildInfo, latency: Option<&LatencySummary>) -> String {
    let mut w = PromWriter::new();

    w.gauge(
        "kortecx_up",
        "1 when the gateway metrics endpoint is serving.",
        1,
    );

    w.family(
        "kortecx_build_info",
        "gauge",
        "Build metadata; always 1, the version rides the label.",
    );
    w.sample("kortecx_build_info", &[("version", build.version)], 1);

    // --- Rate (cumulative counters; Prometheus derives the per-second rate) ---
    w.counter(
        "kortecx_runs_registered_total",
        "Runs admitted (RunRegistered journal facts).",
        state.runs_registered,
    );
    w.counter(
        "kortecx_motes_proposed_total",
        "Mote placements proposed (Proposed journal facts).",
        state.proposed,
    );
    w.counter(
        "kortecx_motes_committed_total",
        "Mote effects committed durably (Committed journal facts).",
        state.committed,
    );

    // --- Errors ---
    w.counter(
        "kortecx_motes_failed_total",
        "Terminal Mote failures (Failed journal facts).",
        state.failed,
    );
    w.family(
        "kortecx_motes_failed_by_reason_total",
        "counter",
        "Terminal Mote failures bucketed by failure reason.",
    );
    for (i, label) in FAILURE_REASON_LABELS.iter().enumerate() {
        w.sample(
            "kortecx_motes_failed_by_reason_total",
            &[("reason", label)],
            state.failed_by_reason[i],
        );
    }
    w.counter(
        "kortecx_motes_repudiated_total",
        "Committed Motes later invalidated (Repudiated journal facts).",
        state.repudiated,
    );
    w.counter(
        "kortecx_effects_staged_total",
        "WORLD-MUTATING effects staged (EffectStaged journal facts).",
        state.effect_staged,
    );
    if let Some(bp) = state.success_ratio_bp() {
        w.gauge(
            "kortecx_success_ratio_basis_points",
            "Committed / (committed + failed), in basis points (0..=10000).",
            bp,
        );
    }

    // --- Duration (operational recent-window latency from the telemetry exhaust) ---
    // Distinct p50/p95 GAUGE families (not a `quantile`-labelled series): these are
    // a point-in-time recent-window snapshot, not a cumulative `summary`, and the
    // `quantile` label is reserved for summary-typed metrics in the OpenMetrics model
    // (a strict parser rejects it on a gauge). Honest types > clever labels.
    if let Some(l) = latency {
        w.gauge(
            "kortecx_mote_wall_p50_ms",
            "Recent-window p50 per-Mote wall-clock latency (ms; model motes).",
            l.p50_ms,
        );
        w.gauge(
            "kortecx_mote_wall_p95_ms",
            "Recent-window p95 per-Mote wall-clock latency (ms; model motes).",
            l.p95_ms,
        );
        w.gauge(
            "kortecx_telemetry_window_motes",
            "Number of recent model Motes the latency window covers.",
            l.window,
        );
        w.gauge(
            "kortecx_output_tokens_window",
            "Summed output_tokens over the recent telemetry window.",
            l.output_tokens,
        );
    }

    // --- The journal high-water mark (a liveness/progress gauge) ---
    w.gauge(
        "kortecx_journal_seq",
        "Highest journal sequence folded into these metrics.",
        state.last_seq,
    );

    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUILD: BuildInfo = BuildInfo { version: "9.9.9" };

    fn sample_state() -> MetricsState {
        let mut s = MetricsState::new();
        s.runs_registered = 3;
        s.proposed = 7;
        s.committed = 5;
        s.failed = 2;
        s.failed_by_reason[0] = 1; // timed_out
        s.failed_by_reason[8] = 1; // dead_lettered
        s.repudiated = 0;
        s.effect_staged = 4;
        s.last_seq = 21;
        s
    }

    #[test]
    fn renders_counters_without_latency() {
        let out = render(&sample_state(), &BUILD, None);
        assert!(out.contains("# TYPE kortecx_motes_committed_total counter"));
        assert!(out.contains("kortecx_motes_committed_total 5"));
        assert!(out.contains("kortecx_motes_failed_by_reason_total{reason=\"timed_out\"} 1"));
        assert!(out.contains("kortecx_motes_failed_by_reason_total{reason=\"dead_lettered\"} 1"));
        assert!(out.contains("kortecx_build_info{version=\"9.9.9\"} 1"));
        assert!(out.contains("kortecx_success_ratio_basis_points 7142")); // 5/7 * 10000
        assert!(out.contains("kortecx_journal_seq 21"));
        // No latency block when latency is absent (honest omit, not zeros).
        assert!(!out.contains("kortecx_mote_wall_p50_ms"));
        assert!(!out.contains("kortecx_mote_wall_p95_ms"));
        assert!(out.contains("kortecx_up 1"));
    }

    #[test]
    fn renders_latency_block_when_supplied() {
        let latency = LatencySummary {
            window: 100,
            p50_ms: 42,
            p95_ms: 130,
            output_tokens: 9001,
        };
        let out = render(&sample_state(), &BUILD, Some(&latency));
        assert!(out.contains("# TYPE kortecx_mote_wall_p50_ms gauge"));
        assert!(out.contains("kortecx_mote_wall_p50_ms 42"));
        assert!(out.contains("kortecx_mote_wall_p95_ms 130"));
        assert!(out.contains("kortecx_telemetry_window_motes 100"));
        assert!(out.contains("kortecx_output_tokens_window 9001"));
    }

    #[test]
    fn render_is_deterministic() {
        let s = sample_state();
        assert_eq!(render(&s, &BUILD, None), render(&s, &BUILD, None));
    }

    #[test]
    fn label_values_are_escaped() {
        let mut buf = String::new();
        escape_label_value(&mut buf, "a\"b\\c");
        assert_eq!(buf, "a\\\"b\\\\c");
    }

    #[test]
    fn empty_state_renders_zero_counters_and_no_ratio() {
        let out = render(&MetricsState::new(), &BUILD, None);
        assert!(out.contains("kortecx_motes_committed_total 0"));
        // No terminal outcomes ⇒ no success-ratio gauge (honest absence).
        assert!(!out.contains("kortecx_success_ratio_basis_points"));
    }
}
