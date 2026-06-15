/**
 * The Dashboard — an at-a-glance landing over data the gateway ALREADY exposes.
 * Distinct from Monitoring (the deep telemetry view) and Activity (the run-scoped
 * drawer): this is the operator's home, an honest overview with NO fabricated
 * numbers (GR15). Every KPI traces to a real RPC — runs (`ListRuns`), output
 * tokens + p50 latency (`ListMoteTelemetry`, over the loaded window), and the
 * count of models backing the live loop (`ListModels`). The reference app's
 * fabricated cards (active-agents, tasks-today, sparklines, cost, success-rate,
 * per-provider latency) are deliberately OMITTED rather than faked.
 *
 * Pure renderer: composes existing hooks + the shared `MetricCard`, `GlobalFeed`
 * and `HealthIndicator`. Added as a Workspace nav item; `/` still lands on Chat
 * (D137) — the default route is unchanged.
 */

import { Link } from "@tanstack/react-router";
import { m } from "framer-motion";
import { stagger } from "../../app/motion";
import { useModels } from "../../kx/use-models";
import { useRuns } from "../../kx/use-runs";
import { useTelemetry } from "../../kx/use-telemetry";
import { shortHex } from "../../lib/format";
import { summarizeRuns, wallClockPercentiles } from "../../lib/monitoring";
import { EmptyState } from "../EmptyState";
import { GlobalFeed } from "../activity/GlobalFeed";
import { GlowCard } from "../ds/GlowCard";
import { HealthIndicator } from "../metrics/HealthIndicator";
import { MetricCard } from "../metrics/MetricCard";

/** The quick-start targets — pure navigation, no data (never implies a capability
 *  we lack). Each lands on a REAL section. */
const QUICK_START = [
  { to: "/chat", title: "Start a chat", hint: "Agentic conversation over the runtime" },
  { to: "/recipes", title: "Run a blueprint", hint: "Catalog & invoke a workflow" },
  { to: "/datasets", title: "Ground with datasets", hint: "Ingest & search RAG corpora" },
  { to: "/monitor", title: "Open Monitoring", hint: "Telemetry & self-correction trails" },
] as const;

export function DashboardSection() {
  const runs = useRuns();
  const telemetry = useTelemetry();
  const models = useModels();

  const runRollup = summarizeRuns(runs.runs);
  const wall = wallClockPercentiles(telemetry.rows);
  const servingCount = models.models?.filter((mdl) => mdl.serving).length;
  const recent = runs.runs.slice(0, 6);

  return (
    <section className="screen" data-testid="dashboard-section">
      <div className="section-head">
        <div>
          <h1>Dashboard</h1>
          <p className="muted">
            A live at-a-glance overview of this gateway — runs, throughput &amp; health.
          </p>
        </div>
        <HealthIndicator />
      </div>

      <m.div
        className="metrics-grid"
        variants={stagger()}
        initial="hidden"
        animate="show"
        data-testid="dashboard-kpis"
      >
        <MetricCard label="Runs" value={runRollup.total} tone="committed" />
        <MetricCard
          label="Output tokens"
          value={wall.totalOutputTokens}
          tone="info"
          sub={`over last ${wall.count} motes`}
        />
        <MetricCard
          label="p50 wall ms"
          value={wall.p50WallMs}
          tone="neutral"
          sub={`over last ${wall.count} motes`}
        />
        <MetricCard
          label="Serving models"
          value={servingCount ?? "—"}
          tone="scheduled"
          sub={models.unsupported ? "not wired here" : "back the live loop"}
        />
      </m.div>

      <div className="dashboard-cols">
        <GlowCard hover={false} className="monitor-panel" data-testid="dashboard-recent">
          <div className="monitor-panel__head">
            <h2>Recent runs</h2>
            <Link to="/workflows" className="linkbtn">
              All workflows
            </Link>
          </div>
          {recent.length === 0 ? (
            <EmptyState
              title="No runs yet"
              detail="Invoke a blueprint or start a chat — runs show up here as they register."
            />
          ) : (
            <ul className="dashboard-runs">
              {recent.map((r) => (
                <li className="dashboard-runs__row" key={r.instanceId} data-testid="dashboard-run">
                  <Link
                    to="/workflows/$instanceId"
                    params={{ instanceId: r.instanceId }}
                    className="dashboard-runs__handle linkbtn"
                    title="Open this run"
                  >
                    {r.handle ?? shortHex(r.instanceId)}
                  </Link>
                  <span className="dashboard-runs__time">
                    {r.startedAt > 0 ? new Date(r.startedAt).toLocaleTimeString() : "—"}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </GlowCard>

        <GlowCard hover={false} className="monitor-panel" data-testid="dashboard-feed">
          <div className="monitor-panel__head">
            <h2>Live activity</h2>
            <span className="muted">newest first</span>
          </div>
          <GlobalFeed />
        </GlowCard>
      </div>

      <div className="quickstart-grid" data-testid="dashboard-quickstart">
        {QUICK_START.map((q) => (
          <Link key={q.to} to={q.to} className="quickstart-card">
            <span className="quickstart-card__title">{q.title}</span>
            <span className="quickstart-card__hint">{q.hint}</span>
          </Link>
        ))}
      </div>
    </section>
  );
}
