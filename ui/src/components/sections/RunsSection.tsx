import { Link } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useMemo, useState } from "react";
import { rowEntrance } from "../../app/motion";
import { useRuns } from "../../kx/use-runs";
import { shortHex } from "../../lib/format";
import { EmptyState } from "../EmptyState";

/**
 * Run history — durable runs enumerated from the journal (`ListRuns`, UI-2) merged
 * with this session's per-invocation records. Filter/search by id or handle; on a
 * gateway without `ListRuns` it degrades to the session-only history.
 */
export function RunsSection() {
  const { runs, clear, notWired, hasMore, loadMore, refresh } = useRuns();
  const [filter, setFilter] = useState("");

  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (q === "") {
      return runs;
    }
    return runs.filter(
      (r) => r.instanceId.toLowerCase().includes(q) || (r.handle ?? "").toLowerCase().includes(q),
    );
  }, [runs, filter]);

  return (
    <section className="screen" data-testid="runs-section">
      <div className="screen__head">
        <h1>Runs</h1>
        <div className="screen__actions">
          <button type="button" className="linkbtn" onClick={refresh}>
            Refresh
          </button>
          {runs.length > 0 ? (
            <button type="button" className="linkbtn" onClick={clear}>
              Clear session
            </button>
          ) : null}
        </div>
      </div>
      <p className="muted">
        {notWired
          ? "This gateway does not enumerate runs — showing this session's history only."
          : "Durable runs from the journal, merged with this session's invocations."}
      </p>

      {runs.length > 0 ? (
        <input
          className="filter-input"
          data-testid="runs-filter"
          placeholder="Filter by id or blueprint…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          spellCheck={false}
          autoComplete="off"
          aria-label="Filter runs"
        />
      ) : null}

      {runs.length === 0 ? (
        <EmptyState
          title="No runs yet"
          detail="Submit a blueprint from the Blueprints section to start one."
        />
      ) : shown.length === 0 ? (
        <EmptyState title="No matching runs" detail="Adjust the filter above." />
      ) : (
        <ul className="run-list" data-testid="run-list">
          {shown.map((r, i) => (
            <m.li
              className="run-list__item card-hover"
              key={`${r.instanceId}:${r.terminalMoteId ?? ""}`}
              {...rowEntrance(i)}
            >
              <Link
                to="/workflows/$instanceId"
                params={{ instanceId: r.instanceId }}
                search={r.terminalMoteId ? { terminal: r.terminalMoteId } : {}}
                className="run-list__link mono"
              >
                {shortHex(r.instanceId)}
              </Link>
              <span className="muted">{r.handle ?? "run"}</span>
              <span className="muted">{new Date(r.startedAt).toLocaleTimeString()}</span>
            </m.li>
          ))}
        </ul>
      )}

      {hasMore ? (
        <button type="button" className="linkbtn" data-testid="runs-load-more" onClick={loadMore}>
          Load more
        </button>
      ) : null}
    </section>
  );
}
