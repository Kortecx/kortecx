/**
 * Monitoring — the gateway-WIDE telemetry dashboard. Distinct from Activity (which is
 * run-scoped): this folds cross-run signals the gateway already exposes — run counts,
 * the self-correction trails (`ListReplanRounds` / `ListReactTurns`), the Morphic
 * action-capture stream (`ListCaptureRecords`), and liveness. Pure renderer: every
 * number comes from `lib/monitoring.ts` over the hooks; each panel degrades to an
 * honest "not wired here" note when its RPC is unimplemented (never a hollow placeholder).
 */

import { m } from "framer-motion";
import { fadeUp, stagger } from "../../app/motion";
import { useCaptureRecords } from "../../kx/use-capture-records";
import { useReactTurns } from "../../kx/use-react-turns";
import { useReplanRounds } from "../../kx/use-replan-rounds";
import { useRuns } from "../../kx/use-runs";
import { shortHex } from "../../lib/format";
import {
  type Tally,
  summarizeCaptures,
  summarizeReact,
  summarizeReplan,
  summarizeRuns,
  tallyRows,
} from "../../lib/monitoring";
import { GlowCard } from "../ds/GlowCard";
import { HealthIndicator } from "../metrics/HealthIndicator";
import { MetricCard } from "../metrics/MetricCard";

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

export function MonitoringSection() {
  const runs = useRuns();
  const replan = useReplanRounds();
  const react = useReactTurns();
  const capture = useCaptureRecords();

  const runRollup = summarizeRuns(runs.runs);
  const replanSummary = summarizeReplan(replan.rounds);
  const reactSummary = summarizeReact(react.turns);
  const captureSummary = summarizeCaptures(capture.records);

  function refreshAll(): void {
    runs.refresh();
    void replan.refetch();
    void react.refetch();
    void capture.refetch();
  }

  return (
    <section className="screen" data-testid="monitoring-section">
      <div className="section-head">
        <div>
          <h1>Monitoring</h1>
          <p className="muted">
            Gateway-wide metrics, self-correction trails & the action-capture stream.
          </p>
        </div>
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
                {capture.records.slice(0, 10).map((r) => (
                  <tr key={`${r.seq}-${r.moteId}`}>
                    <td className="mono">{shortHex(r.moteId)}</td>
                    <td className="mono">{shortHex(r.resultRef)}</td>
                    <td className="mono">{r.ndClass || "—"}</td>
                    <td className="mono">#{r.seq}</td>
                  </tr>
                ))}
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
    </section>
  );
}
