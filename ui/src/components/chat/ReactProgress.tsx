/**
 * The agent task-loop's live progress strip (PR-2.1): one chip per ReAct turn
 * — pending (thinking) · tool (acting, with the fired tool's id@version) ·
 * answer · dead-lettered — narrated from the chain's DURABLE ReactRound facts
 * (`ListReactTurns`), so what renders is exactly what the journal committed.
 */

import type { ReactTurnVM } from "../../kx/use-react-progress";

function chipLabel(t: ReactTurnVM): string {
  switch (t.branch) {
    case "tool":
      return `turn ${t.turn}: tool ${t.toolId}@${t.toolVersion}`;
    case "answer":
      return `turn ${t.turn}: answer`;
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
  return (
    <div className="react-progress" data-testid="react-progress">
      <span className="muted">
        Agent loop ({turns.length}/{cap} turns):
      </span>
      {turns.map((t) => (
        <span
          key={t.turn}
          className="badge"
          data-branch={t.branch}
          data-testid={`react-turn-${t.turn}`}
        >
          {chipLabel(t)}
        </span>
      ))}
    </div>
  );
}
