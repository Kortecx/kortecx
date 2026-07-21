/**
 * WAVE-3 (PR-1): the App Run interface's WATCH surface — the run's ReAct chain as a
 * turn-by-turn timeline of cards, narrated from the DURABLE ReactRound facts
 * (`ListReactTurns` via {@link useReactProgress}) so what renders is exactly what the
 * journal committed. Each card = one turn: a high-level step-type badge (Model / Tool /
 * MCP / Connector via `lib/step-kind`), the branch (thinking · tool `id@version` ·
 * answered · rejected · dead-lettered), and a rejected turn's fail-closed reason as a
 * keyboard-toggleable disclosure. A pure-DAG (non-agent) run has NO ReAct turns, so the
 * tab honestly falls back to a compact per-Mote step list from the projection + step
 * kinds — never a blank tab. Read-only / display-only (SN-8): NO new RPC — reuses the
 * shipped `useReactProgress` + `useRunStepKinds` + `GetProjection`. REVIEW (the run's
 * committed outputs) rides the same tab via {@link RunChanges}.
 */

import { type ProjectionVM, allTerminal } from "../../kx/use-projection";
import { type ReactTurnVM, useReactProgress } from "../../kx/use-react-progress";
import { useRunStepKinds } from "../../kx/use-run-step-kinds";
import { STEP_LABEL, type StepType, classifyStep } from "../../lib/step-kind";
import { RunChanges } from "./RunChanges";

/** The badge step-type for a turn: a `tool` turn is classified from its fired tool
 *  (no GetMoteDetail — cheaper + honest); every other branch is the model reasoning /
 *  answer turn. */
function turnStepType(t: ReactTurnVM): StepType {
  return t.branch === "tool" ? classifyStep("TOOL", { [t.toolId]: "" }) : "model";
}

/** A short human phrase for the turn's branch (the card's status line). */
function branchLabel(t: ReactTurnVM): string {
  switch (t.branch) {
    case "tool":
      return `${t.toolId}@${t.toolVersion}`;
    case "answer":
      return "answered";
    case "rejected":
      return "tool proposal rejected";
    case "dead_lettered":
      return "dead-lettered";
    default:
      return "thinking…";
  }
}

function TurnCard({ turn }: { turn: ReactTurnVM }) {
  const kind = turnStepType(turn);
  const reason = turn.branch === "rejected" && turn.rejectionReason ? turn.rejectionReason : "";
  return (
    <li className="run-timeline__card" data-testid={`run-turn-${turn.turn}`}>
      <div className="run-timeline__card-head">
        <span className="run-timeline__turn">Turn {turn.turn}</span>
        <span className="badge" data-branch={turn.branch}>
          {STEP_LABEL[kind]}
        </span>
        <span className="run-timeline__branch">{branchLabel(turn)}</span>
      </div>
      {reason ? (
        <details className="run-timeline__reason">
          <summary>why it re-prompted</summary>
          <span className="muted" data-testid={`run-turn-${turn.turn}-reason`}>
            {reason}
          </span>
        </details>
      ) : null}
    </li>
  );
}

/** The pure-DAG fallback: a run with no ReAct chain (a plain workflow) still shows its
 *  committed steps, labelled by high-level type — so the timeline tab is never blank. */
function StepFallback({
  instanceId,
  projection,
}: {
  instanceId: string;
  projection: ProjectionVM;
}) {
  const kinds = useRunStepKinds(instanceId, projection.motes);
  const committed = projection.motes.filter((m) => m.moteDefHash !== "");
  if (committed.length === 0) {
    return <p className="muted">No committed steps yet.</p>;
  }
  return (
    <ol className="run-timeline__turns" data-testid="run-timeline-steps">
      {committed.map((m, i) => (
        <li key={m.moteId} className="run-timeline__card" data-testid={`run-step-${m.moteId}`}>
          <div className="run-timeline__card-head">
            <span className="run-timeline__turn">Step {i + 1}</span>
            <span className="badge">{STEP_LABEL[kinds.get(m.moteId) ?? "unknown"]}</span>
          </div>
        </li>
      ))}
    </ol>
  );
}

export function RunTimeline({
  instanceId,
  projection,
  chainSalt,
}: {
  instanceId: string;
  projection: ProjectionVM;
  /** The per-submission chain key. `ListReactTurns` is scoped by instance alone
   *  otherwise, and a serve shares ONE instance across every run — so without this the
   *  timeline listed every agentic turn in the journal, not this run's. Chat already
   *  passes it (`use-chat.ts`); this view did not. */
  chainSalt?: string;
}) {
  const { turns, terminal } = useReactProgress(instanceId, chainSalt);
  const settled = terminal !== null || allTerminal(projection);
  const isAgentRun = turns.length > 0;
  return (
    <div className="run-timeline" data-testid="run-timeline">
      <div className="run-timeline__status">
        <span className={`run-timeline__pulse${settled ? " is-rest" : ""}`} aria-hidden="true" />
        <span className="muted">
          {isAgentRun
            ? `Agent loop — ${turns.length} turn${turns.length === 1 ? "" : "s"} · ${settled ? "at rest" : "live"}`
            : settled
              ? "Run at rest"
              : "Run in progress…"}
        </span>
      </div>
      {isAgentRun ? (
        <ol className="run-timeline__turns">
          {turns.map((t) => (
            <TurnCard key={`${t.turn}:${t.callIndex}`} turn={t} />
          ))}
        </ol>
      ) : settled ? (
        <StepFallback instanceId={instanceId} projection={projection} />
      ) : (
        <p className="muted">Agent loop starting…</p>
      )}
      <RunChanges instanceId={instanceId} projection={projection} />
    </div>
  );
}
