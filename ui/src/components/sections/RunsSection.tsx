import { Link } from "@tanstack/react-router";
import { useRuns } from "../../kx/use-runs";
import { shortHex } from "../../lib/format";
import { EmptyState } from "../EmptyState";

/**
 * Session run history (this browser, per endpoint) — the forward seam for the
 * additive `ListRuns` RPC (UI-2 swaps `useRuns`'s source; this view is unchanged).
 */
export function RunsSection() {
  const { runs, clear } = useRuns();
  return (
    <section className="screen" data-testid="runs-section">
      <div className="screen__head">
        <h1>Runs</h1>
        {runs.length > 0 ? (
          <button type="button" className="linkbtn" onClick={clear}>
            Clear history
          </button>
        ) : null}
      </div>
      <p className="muted">
        Runs started from this console. Full server-side history arrives with <code>ListRuns</code>{" "}
        (UI-2).
      </p>
      {runs.length === 0 ? (
        <EmptyState
          title="No runs yet"
          detail="Submit a recipe from the Recipes section to start one."
        />
      ) : (
        <ul className="run-list" data-testid="run-list">
          {runs.map((r) => (
            <li key={r.instanceId} className="run-list__item">
              <Link
                to="/runs/$instanceId"
                params={{ instanceId: r.instanceId }}
                search={r.terminalMoteId ? { terminal: r.terminalMoteId } : {}}
                className="run-list__link mono"
              >
                {shortHex(r.instanceId)}
              </Link>
              <span className="muted">{r.handle ?? "run"}</span>
              <span className="muted">{new Date(r.startedAt).toLocaleTimeString()}</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
