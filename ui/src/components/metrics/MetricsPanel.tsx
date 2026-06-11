import { m } from "framer-motion";
import { stagger } from "../../app/motion";
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
  const metrics = projection ? deriveMetrics(projection) : null;
  return (
    <section className="metrics" data-testid="metrics-panel">
      <m.div className="metrics-grid" variants={stagger()} initial="hidden" animate="show">
        <MetricCard label="Gateway" value={<HealthIndicator />} />
        {metrics ? (
          <>
            <MetricCard label="Motes" value={metrics.total} />
            <MetricCard label="Committed" value={metrics.committed} tone="committed" />
            <MetricCard label="Failed" value={metrics.failed} tone="failed" />
            <MetricCard label="In flight" value={metrics.inFlight} tone="scheduled" />
            <MetricCard label="Success rate" value={asPercent(metrics.successRate)} />
            <MetricCard label="Frontier" value={formatSeq(metrics.currentSeq)} />
            <MetricCard
              label="Commit-seq span"
              value={metrics.latencySeqSpan == null ? "—" : metrics.latencySeqSpan}
            />
          </>
        ) : null}
      </m.div>
      {metrics ? (
        <StateBreakdown metrics={metrics} />
      ) : (
        <p className="muted">Select a run to see its metrics.</p>
      )}
    </section>
  );
}
