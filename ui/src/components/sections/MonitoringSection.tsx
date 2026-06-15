/**
 * Monitoring — the gateway-WIDE telemetry dashboard. Distinct from Activity (which is
 * run-scoped): this folds cross-run signals the gateway already exposes — run counts,
 * the self-correction trails (`ListReplanRounds` / `ListReactTurns`), the Morphic
 * action-capture stream (`ListCaptureRecords`), and liveness. Pure renderer: every
 * number comes from `lib/monitoring.ts` over the hooks; each panel degrades to an
 * honest "not wired here" note when its RPC is unimplemented (never a hollow placeholder).
 *
 * Batch C adds two URL-addressable views beside the overview (the run-detail tab
 * precedent): **Live feed** — the continuous cross-run event tail with run
 * click-through — and **Telemetry** — the host-measured execution exhaust
 * (`ListMoteTelemetry`: wall-clock / model usage / fired tool), cursor-paged.
 */

import { Link } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useMemo } from "react";
import { fadeUp, stagger } from "../../app/motion";
import { useCaptureRecords } from "../../kx/use-capture-records";
import { type RunScopedRef, useResultMapMulti } from "../../kx/use-content-batch";
import { useReactTurns } from "../../kx/use-react-turns";
import { useReplanRounds } from "../../kx/use-replan-rounds";
import { useRuns } from "../../kx/use-runs";
import { useTelemetry } from "../../kx/use-telemetry";
import { shortHex } from "../../lib/format";
import {
  type Tally,
  summarizeCaptures,
  summarizeReact,
  summarizeReplan,
  summarizeRuns,
  summarizeTelemetryByModel,
  tallyRows,
  wallClockPercentiles,
} from "../../lib/monitoring";
import type { MonitorTab } from "../../router/routes/monitor";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { ResultPreview } from "../ResultPreview";
import { GlobalFeed } from "../activity/GlobalFeed";
import { GlowCard } from "../ds/GlowCard";
import { HealthIndicator } from "../metrics/HealthIndicator";
import { MetricCard } from "../metrics/MetricCard";

const MONITOR_VIEWS = [undefined, "feed", "telemetry"] as const;
const VIEW_LABEL: Record<string, string> = {
  overview: "Overview",
  feed: "Live feed",
  telemetry: "Telemetry",
};

function TallyList({ tally, empty }: { tally: Tally; empty: string }) {
  const rows = tallyRows(tally);
  if (rows.length === 0) {
    return <p className="muted">{empty}</p>;
  }
  return (
    <ul className="tally">
      {rows.map(([label, count]) => (
        <li className="tally__row" key={label}>
          <span className="tally__label mono">{label}</span>
          <span className="tally__count">{count}</span>
        </li>
      ))}
    </ul>
  );
}

/** A panel header + body that shows a muted "not wired" note when its RPC is absent. */
function Panel({
  title,
  hint,
  notWired,
  children,
}: {
  title: string;
  hint?: string;
  notWired?: boolean;
  children: React.ReactNode;
}) {
  return (
    <GlowCard hover={false} className="monitor-panel" variants={fadeUp}>
      <div className="monitor-panel__head">
        <h2>{title}</h2>
        {hint ? <span className="muted">{hint}</span> : null}
      </div>
      {notWired ? <p className="muted">Not wired on this gateway.</p> : children}
    </GlowCard>
  );
}

/** Tab state rides the route's validated search params (the run-detail
 *  precedent) — the ROUTE owns the router coupling and passes it down, so the
 *  section stays a pure renderer (component-testable without a router). */
export function MonitoringSection({
  tab,
  onTab,
}: {
  tab?: MonitorTab;
  onTab?: (tab: MonitorTab | undefined) => void;
} = {}) {
  return (
    <section className="screen" data-testid="monitoring-section">
      <div className="section-head">
        <div>
          <h1>Monitoring</h1>
          <p className="muted">
            Gateway-wide metrics, the live cross-run feed & per-mote execution telemetry.
          </p>
        </div>
      </div>

      <fieldset className="view-toggle" aria-label="Monitoring view" data-testid="monitor-tabs">
        {MONITOR_VIEWS.map((v) => (
          <button
            key={v ?? "overview"}
            type="button"
            aria-pressed={tab === v}
            data-testid={`monitor-tab-${v ?? "overview"}`}
            onClick={() => onTab?.(v)}
          >
            {VIEW_LABEL[v ?? "overview"]}
          </button>
        ))}
      </fieldset>

      {tab === "feed" ? <FeedView /> : tab === "telemetry" ? <TelemetryView /> : <OverviewView />}
    </section>
  );
}

/** The Batch C continuous cross-run feed — one GlowCard hosting the same
 *  GlobalFeed the Activity drawer lands on; rows link to the run detail. */
function FeedView() {
  return (
    <GlowCard hover={false} className="monitor-panel" data-testid="monitor-feed">
      <div className="monitor-panel__head">
        <h2>Live feed</h2>
        <span className="muted">every run on this node, newest first</span>
      </div>
      <GlobalFeed />
    </GlowCard>
  );
}

/** The Batch C execution-telemetry table: host-measured wall-clock + model
 *  usage + the fired tool per committed mote, cursor-paged newest-first. */
function TelemetryView() {
  const t = useTelemetry();
  // Client-side rollups over the LOADED telemetry window (cursor-paged, so this is
  // "the last N motes on this page", NOT all-time — labeled honestly below).
  const byModel = useMemo(() => summarizeTelemetryByModel(t.rows), [t.rows]);
  const wall = useMemo(() => wallClockPercentiles(t.rows), [t.rows]);
  return (
    <GlowCard hover={false} className="monitor-panel" data-testid="monitor-telemetry">
      <div className="monitor-panel__head">
        <h2>Execution telemetry</h2>
        <span className="muted">wall-clock · model usage · fired tool (audit-only exhaust)</span>
      </div>
      {t.notWired ? (
        <p className="muted">
          Not wired on this gateway — telemetry needs a Batch-C serve with its telemetry.db sidecar
          (upgrade the serve to see execution metrics).
        </p>
      ) : t.error ? (
        <ErrorNotice error={t.error} />
      ) : t.isLoading ? (
        <EmptyState title="Loading telemetry…" />
      ) : t.rows.length === 0 ? (
        <EmptyState
          title="No telemetry yet"
          detail="Rows appear as motes execute and commit — invoke a recipe to generate some."
        />
      ) : (
        <>
          <m.div
            className="metrics-grid"
            variants={stagger()}
            initial="hidden"
            animate="show"
            data-testid="telemetry-kpis"
          >
            <MetricCard label="Motes" value={byModel.windowSize} tone="neutral" sub="this page" />
            <MetricCard
              label="p50 wall ms"
              value={wall.p50WallMs}
              tone="info"
              sub={`over last ${wall.count}`}
            />
            <MetricCard
              label="p95 wall ms"
              value={wall.p95WallMs}
              tone="info"
              sub={`over last ${wall.count}`}
            />
            <MetricCard
              label="Output tokens"
              value={wall.totalOutputTokens}
              tone="committed"
              sub="this page"
            />
            {/* Honest-disabled: OSS serves locally — no price / input_tokens / expert
                entity to bill. Per-expert cost arrives with managed Cloud (D129). */}
            <div className="metric-card metric-card--disabled" data-testid="cost-tile-disabled">
              <span className="metric-card__value">
                <span className="chip--soon">Cloud</span>
              </span>
              <span className="metric-card__label">Cost &amp; per-expert billing</span>
              <span className="metric-card__sub">No cost/input-token data in OSS (D129).</span>
            </div>
          </m.div>

          {byModel.rows.length > 0 ? (
            <>
              <div className="monitor-panel__head">
                <h3>Per-model rollup</h3>
                <span className="muted">
                  over the last {byModel.windowSize} motes (this page, not all-time)
                </span>
              </div>
              <table className="trail-table" data-testid="telemetry-by-model">
                <thead>
                  <tr>
                    <th>Model</th>
                    <th>Count</th>
                    <th>p50&nbsp;ms</th>
                    <th>p95&nbsp;ms</th>
                    <th>Out&nbsp;tokens</th>
                  </tr>
                </thead>
                <tbody>
                  {byModel.rows.map((r) => (
                    <tr key={r.modelId} data-testid="telemetry-by-model-row">
                      <td className="mono">{r.modelId}</td>
                      <td className="mono">{r.count}</td>
                      <td className="mono">{r.p50WallMs}</td>
                      <td className="mono">{r.p95WallMs}</td>
                      <td className="mono">{r.totalOutputTokens}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          ) : null}

          <table className="trail-table" data-testid="telemetry-table">
            <thead>
              <tr>
                <th>Mote</th>
                <th>Run</th>
                <th>Model</th>
                <th>Tool</th>
                <th>Out&nbsp;tokens</th>
                <th>Wall&nbsp;ms</th>
                <th>Started</th>
                <th>seq</th>
              </tr>
            </thead>
            <tbody>
              {t.rows.map((r) => (
                <tr key={`${r.seq}-${r.moteId}`} data-testid="telemetry-row">
                  <td className="mono">{shortHex(r.moteId)}</td>
                  <td className="mono">
                    {r.instanceId ? (
                      <Link
                        to="/workflows/$instanceId"
                        params={{ instanceId: r.instanceId }}
                        className="linkbtn mono"
                        title="Open this run"
                      >
                        {shortHex(r.instanceId)}
                      </Link>
                    ) : (
                      "—"
                    )}
                  </td>
                  <td className="mono">{r.modelId || "—"}</td>
                  <td className="mono">{r.toolId || "—"}</td>
                  <td className="mono">{r.outputTokens ?? "—"}</td>
                  <td className="mono">{r.wallClockMs}</td>
                  <td className="muted">
                    {r.startedUnixMs > 0 ? new Date(r.startedUnixMs).toLocaleTimeString() : "—"}
                  </td>
                  <td className="mono">#{r.seq}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {t.hasMore ? (
            <button
              type="button"
              className="linkbtn"
              onClick={t.loadMore}
              disabled={t.isLoadingMore}
              data-testid="telemetry-load-more"
            >
              {t.isLoadingMore ? "Loading…" : "Load more"}
            </button>
          ) : null}
        </>
      )}
    </GlowCard>
  );
}

/** The pre-Batch-C overview panels, unchanged. */
function OverviewView() {
  const runs = useRuns();
  const replan = useReplanRounds();
  const react = useReactTurns();
  const capture = useCaptureRecords();
  const telemetry = useTelemetry({ pageSize: 1 });

  const runRollup = summarizeRuns(runs.runs);
  const replanSummary = summarizeReplan(replan.rounds);
  const reactSummary = summarizeReact(react.turns);
  const captureSummary = summarizeCaptures(capture.records);
  // Resolve the (bounded, 10-row) capture table's results to TEXT, grouped by
  // run (the records span runs; GetContentBatch is run-scoped). Telemetry stays
  // exhaust — but the Result is the headline here, not a bare hash (D142.2).
  const capturePairs = useMemo<RunScopedRef[]>(
    () =>
      capture.records
        .slice(0, 10)
        .flatMap((r) =>
          r.resultRef && r.instanceId ? [{ instanceId: r.instanceId, ref: r.resultRef }] : [],
        ),
    [capture.records],
  );
  const captureResults = useResultMapMulti(capturePairs);

  function refreshAll(): void {
    runs.refresh();
    void replan.refetch();
    void react.refetch();
    void capture.refetch();
    telemetry.refetch();
  }

  return (
    <>
      <div className="section-head">
        <div />
        <button type="button" className="linkbtn" onClick={refreshAll}>
          Refresh
        </button>
      </div>
      <m.div className="metrics-grid" variants={stagger()} initial="hidden" animate="show">
        <MetricCard label="Runs" value={runRollup.total} tone="committed" />
        <MetricCard label="Re-plan rounds" value={replanSummary.total} tone="scheduled" />
        <MetricCard label="ReAct turns" value={reactSummary.total} />
        <MetricCard label="Tool calls" value={reactSummary.toolCalls} />
        <MetricCard label="Captured actions" value={captureSummary.total} />
        <MetricCard
          label="Last mote wall ms"
          value={telemetry.rows[0] ? telemetry.rows[0].wallClockMs : "—"}
        />
      </m.div>

      <m.div className="monitor-grid" variants={stagger()} initial="hidden" animate="show">
        <Panel title="Runs" hint={`${runRollup.total} total`}>
          <TallyList tally={runRollup.byHandle} empty="No runs recorded yet." />
        </Panel>

        <Panel
          title="Self-correction"
          hint={`${replanSummary.total} rounds · ${replanSummary.escalated} escalated`}
          notWired={replan.notWired}
        >
          <p className="muted">
            {replanSummary.failedStepCount} failed step
            {replanSummary.failedStepCount === 1 ? "" : "s"} triggered re-plans.
          </p>
          <TallyList tally={replanSummary.byModel} empty="No re-plan rounds yet." />
          {replan.rounds.length > 0 ? (
            <table className="trail-table">
              <thead>
                <tr>
                  <th>#</th>
                  <th>Shaper</th>
                  <th>Model</th>
                  <th>Escalated</th>
                </tr>
              </thead>
              <tbody>
                {replan.rounds.slice(0, 8).map((r) => (
                  <tr key={`${r.seq}-${r.round}`}>
                    <td className="mono">{r.round}</td>
                    <td className="mono">{shortHex(r.shaperMoteId)}</td>
                    <td className="mono">{r.modelId || "—"}</td>
                    <td>{r.escalated ? "⚠" : "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : null}
        </Panel>

        <Panel
          title="ReAct turns"
          hint={`${reactSummary.toolCalls} tool calls`}
          notWired={react.notWired}
        >
          <TallyList tally={reactSummary.byBranch} empty="No ReAct turns yet." />
          {react.turns.length > 0 ? (
            <table className="trail-table">
              <thead>
                <tr>
                  <th>#</th>
                  <th>Branch</th>
                  <th>Tool</th>
                </tr>
              </thead>
              <tbody>
                {react.turns.slice(0, 8).map((t) => (
                  <tr key={`${t.seq}-${t.turn}`}>
                    <td className="mono">{t.turn}</td>
                    <td>{t.branch || "—"}</td>
                    <td className="mono">{t.toolId ? `${t.toolId}@${t.toolVersion}` : "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : null}
        </Panel>

        <Panel
          title="Action capture"
          hint={`${captureSummary.total} records`}
          notWired={capture.notWired}
        >
          <TallyList tally={captureSummary.byNdClass} empty="No captured actions yet." />
          {capture.records.length > 0 ? (
            <table className="trail-table">
              <thead>
                <tr>
                  <th>Mote</th>
                  <th>Result</th>
                  <th>nd_class</th>
                  <th>seq</th>
                </tr>
              </thead>
              <tbody>
                {capture.records.slice(0, 10).map((r) => {
                  const vm = r.resultRef ? captureResults.byRef.get(r.resultRef) : undefined;
                  return (
                    <tr key={`${r.seq}-${r.moteId}`}>
                      <td className="mono">{shortHex(r.moteId)}</td>
                      <td className="trail-table__result">
                        <ResultPreview
                          resultRef={r.resultRef || null}
                          content={vm?.content}
                          missing={vm?.missing ?? false}
                          loading={captureResults.isLoading}
                          max={60}
                        />
                      </td>
                      <td className="mono">{r.ndClass || "—"}</td>
                      <td className="mono">#{r.seq}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          ) : null}
        </Panel>

        <Panel title="Gateway health">
          <div className="monitor-health">
            <HealthIndicator />
          </div>
        </Panel>
      </m.div>
    </>
  );
}
