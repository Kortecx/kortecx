/**
 * The agent task-loop's live progress strip (PR-2.1): one chip per ReAct turn
 * — pending (thinking) · tool (acting, with the fired tool's id@version) ·
 * answer · rejected (PR-3/A2: a refused tool proposal the model re-prompts over;
 * the reason is a click-to-expand disclosure) · dead-lettered — narrated from
 * the chain's DURABLE ReactRound facts (`ListReactTurns`), so what renders is
 * exactly what the journal committed.
 */

import type { ReactTurnVM } from "../../kx/use-react-progress";

function chipLabel(t: ReactTurnVM): string {
  switch (t.branch) {
    case "tool":
      return `turn ${t.turn}: tool ${t.toolId}@${t.toolVersion}`;
    case "answer":
      return `turn ${t.turn}: answer`;
    case "rejected":
      return `turn ${t.turn}: rejected`;
    case "dead_lettered":
      return `turn ${t.turn}: dead-lettered`;
    default:
      return `turn ${t.turn}: thinking…`;
  }
}

export function ReactProgress({ turns }: { turns: readonly ReactTurnVM[] }) {
  if (turns.length === 0) {
    return (
      <p className="muted react-progress" data-testid="react-progress">
        Agent loop starting…
      </p>
    );
  }
  const cap = turns[0]?.maxTurns ?? 8;
  // PR-9c-1: the AUDITED action set — the chain's settled `tool` turns. A pure
  // derivation over the same durable facts (no new RPC/state); shown as an honest
  // summary so the action set reads as a *set*, not only per-turn chips.
  const actions = turns.filter((t) => t.branch === "tool");
  const distinctTools = [...new Set(actions.map((t) => `${t.toolId}@${t.toolVersion}`))];
  // W2: a chain that dead-lettered AFTER firing tools looped on its tool budget
  // without ever settling on an answer (vs an all-rejected dead-letter, whose
  // per-turn reasons already explain it). A pure derivation over the same facts.
  const loopedOnTools = turns.some((t) => t.branch === "dead_lettered") && actions.length > 0;
  // Governance observability: the chain's run-fixed warrant axes (names/refs only) —
  // what this run may fire + which secrets it may resolve. Chain-level, so the first row
  // carrying them is representative. Makes a dropped capability axis visible, not silent.
  const grants = turns.find((t) => t.grantedTools.length > 0 || t.secretScopeNames.length > 0);
  return (
    <div className="react-progress" data-testid="react-progress">
      <span className="muted">
        Agent loop ({turns.length}/{cap} turns):
      </span>
      {turns.map((t) =>
        // PR-3 (A2): a rejected turn is a native disclosure — the chip is the
        // summary (keyboard-toggleable, both themes via .badge tokens), the
        // fail-closed reason expands below it so an operator sees WHY the model
        // re-prompted (or, at budget exhaustion, why the chain finally died).
        t.branch === "rejected" && t.rejectionReason ? (
          <details
            key={t.turn}
            className="badge react-rejected"
            data-branch={t.branch}
            data-testid={`react-turn-${t.turn}`}
          >
            <summary>{chipLabel(t)}</summary>
            <span className="muted react-reject-reason" data-testid={`react-turn-${t.turn}-reason`}>
              {t.rejectionReason}
            </span>
          </details>
        ) : (
          <span
            key={t.turn}
            className="badge"
            data-branch={t.branch}
            data-testid={`react-turn-${t.turn}`}
          >
            {chipLabel(t)}
          </span>
        ),
      )}
      {actions.length > 0 && (
        <span className="muted react-actions" data-testid="react-actions">
          Actions taken: {actions.length} ({distinctTools.join(", ")})
        </span>
      )}
      {grants && (
        <span className="muted react-grants" data-testid="react-grants">
          Governed by:{" "}
          {grants.grantedTools.length > 0
            ? `tools [${grants.grantedTools.join(", ")}]`
            : "no tools"}
          {grants.secretScopeNames.length > 0
            ? `, secrets [${grants.secretScopeNames.join(", ")}]`
            : ""}
        </span>
      )}
      {loopedOnTools && (
        <span className="muted react-deadletter-hint" data-testid="react-deadletter-hint">
          The agent exhausted its tool-call budget without settling on an answer.
        </span>
      )}
    </div>
  );
}
