import type { ProjectionVM } from "../../kx/use-projection";
import { formatSeq } from "../../lib/format";
import { asPercent, deriveMetrics } from "../../lib/metrics";
import { HealthIndicator } from "./HealthIndicator";
import { MetricCard } from "./MetricCard";
import { StateBreakdown } from "./StateBreakdown";

/**
 * Run observability derived from one run's projection (success/failure rates,
 * in-flight, frontier, and a `committed_seq`-span latency PROXY — not ms; the
 * projection has no wall-clock). Gateway health is always shown. Without a
 * projection, only health + a hint render.
 */
export function MetricsPanel({ projection }: { projection?: ProjectionVM }) {
  const m = projection ? deriveMetrics(projection) : null;
  return (
    <section className="metrics" data-testid="metrics-panel">
      <div className="metrics-grid">
        <MetricCard label="Gateway" value={<HealthIndicator />} />
        {m ? (
          <>
            <MetricCard label="Motes" value={m.total} />
            <MetricCard label="Committed" value={m.committed} tone="committed" />
            <MetricCard label="Failed" value={m.failed} tone="failed" />
            <MetricCard label="In flight" value={m.inFlight} tone="scheduled" />
            <MetricCard label="Success rate" value={asPercent(m.successRate)} />
            <MetricCard label="Frontier" value={formatSeq(m.currentSeq)} />
            <MetricCard
              label="Commit-seq span"
              value={m.latencySeqSpan == null ? "—" : m.latencySeqSpan}
            />
          </>
        ) : null}
      </div>
      {m ? (
        <StateBreakdown metrics={m} />
      ) : (
        <p className="muted">Select a run to see its metrics.</p>
      )}
    </section>
  );
}
