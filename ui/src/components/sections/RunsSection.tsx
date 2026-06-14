import { Link, useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useEffect, useMemo, useState } from "react";
import { rowEntrance } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { useInvoke } from "../../kx/use-invoke";
import { useRecipeNames } from "../../kx/use-recipes";
import { useRuns } from "../../kx/use-runs";
import { shortHex } from "../../lib/format";
import type { RunRecord } from "../../lib/recent-runs";
import { RUN_NAMES_CHANGED_EVENT, loadRunNames, setRunName } from "../../lib/run-names";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";

/**
 * The Workflows home (PR-2.1): durable runs enumerated from the journal
 * (`ListRuns`) merged with this session's per-invocation records — labeled by
 * recipe handle (the fingerprint→handle join) or a client-local rename, with
 * per-row controls: Open · Run again (idempotent re-invoke) · Clone (prefill
 * the Blueprints form). Authoring stays in Blueprints (D141.1) — the "New
 * workflow" button LINKS there, never duplicates it.
 */
export function RunsSection() {
  const { endpoint } = useConnection();
  const { runs, clear, notWired, hasMore, loadMore, refresh } = useRuns();
  const recipeNames = useRecipeNames();
  const [filter, setFilter] = useState("");
  const [names, setNames] = useState<Record<string, string>>(() => loadRunNames(endpoint));

  // Stay fresh across rename events + endpoint switches.
  useEffect(() => {
    setNames(loadRunNames(endpoint));
    function onNamesChanged(): void {
      setNames(loadRunNames(endpoint));
    }
    window.addEventListener(RUN_NAMES_CHANGED_EVENT, onNamesChanged);
    return () => window.removeEventListener(RUN_NAMES_CHANGED_EVENT, onNamesChanged);
  }, [endpoint]);

  const fingerprintNames = recipeNames.data ?? {};

  /** Display name precedence: local rename > session handle > fingerprint join. */
  function nameFor(r: RunRecord): string | null {
    return (
      names[r.instanceId] ??
      r.handle ??
      (r.recipeFingerprint ? (fingerprintNames[r.recipeFingerprint] ?? null) : null)
    );
  }

  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (q === "") {
      return runs;
    }
    return runs.filter(
      (r) =>
        r.instanceId.toLowerCase().includes(q) ||
        (r.handle ?? "").toLowerCase().includes(q) ||
        (names[r.instanceId] ?? "").toLowerCase().includes(q) ||
        (r.recipeFingerprint ? (fingerprintNames[r.recipeFingerprint] ?? "") : "")
          .toLowerCase()
          .includes(q),
    );
  }, [runs, filter, names, fingerprintNames]);

  return (
    <section className="screen" data-testid="runs-section">
      <div className="screen__head">
        <h1>Workflows</h1>
        <div className="screen__actions">
          {/* D141.1: authoring is OPERATED in Blueprints — this links there. */}
          <Link to="/recipes" className="btnlink" data-testid="new-workflow">
            New workflow →
          </Link>
          <button type="button" className="linkbtn" onClick={refresh}>
            Refresh
          </button>
          {runs.length > 0 ? (
            <button
              type="button"
              className="linkbtn"
              data-testid="clear-local-history"
              title="Clears THIS browser's invocation history. Durable runs live in the journal and stay."
              onClick={() => {
                clear();
                refresh();
              }}
            >
              Clear local history
            </button>
          ) : null}
        </div>
      </div>
      <p className="muted">
        {notWired
          ? "This gateway does not enumerate runs — showing this session's history only."
          : "Durable runs from the journal, merged with this session's invocations. Clearing removes only this browser's history — journal runs stay."}
      </p>

      {runs.length > 0 ? (
        <input
          className="filter-input"
          data-testid="runs-filter"
          placeholder="Filter by name, id or blueprint…"
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
          detail="Start one with “New workflow” — pick a blueprint, fill its inputs, and run it."
        />
      ) : shown.length === 0 ? (
        <EmptyState title="No matching runs" detail="Adjust the filter above." />
      ) : (
        <ul className="run-list" data-testid="run-list">
          {shown.map((r, i) => (
            <RunRow
              key={`${r.instanceId}:${r.terminalMoteId ?? ""}`}
              run={r}
              index={i}
              name={nameFor(r)}
            />
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

/** One run row: name + id + time, with Open/Run-again/Clone/Rename controls. */
function RunRow({ run, index, name }: { run: RunRecord; index: number; name: string | null }) {
  const { endpoint } = useConnection();
  const navigate = useNavigate();
  const invoke = useInvoke();
  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState(name ?? "");

  // A durable-only row (recovered from the journal, not started in this browser).
  const journalOnly = run.handle === null && (run.args ?? null) === null;
  const canRunAgain = Boolean(run.handle && run.args);

  function runAgain(): void {
    if (!run.handle || !run.args) {
      return;
    }
    let args: Record<string, unknown>;
    try {
      args = JSON.parse(run.args) as Record<string, unknown>;
    } catch {
      return;
    }
    // Idempotent by construction: the same recipe+args resolves to the same
    // already-committed Mote (the memoizer) — "running again" honestly JOINS it.
    invoke.mutate(
      { handle: run.handle, args },
      {
        onSuccess: ({ instanceId, terminalMoteId }) => {
          void navigate({
            to: "/workflows/$instanceId",
            params: { instanceId },
            search: { terminal: terminalMoteId },
          });
        },
      },
    );
  }

  function saveRename(): void {
    setRunName(endpoint, run.instanceId, draft);
    setRenaming(false);
  }

  return (
    <m.li className="run-list__item card-hover" {...rowEntrance(index)}>
      <div className="run-list__main">
        <Link
          to="/workflows/$instanceId"
          params={{ instanceId: run.instanceId }}
          search={run.terminalMoteId ? { terminal: run.terminalMoteId } : {}}
          className="run-list__link"
          data-testid="run-open"
        >
          {renaming ? null : (
            <span className="run-list__name">{name ?? shortHex(run.instanceId)}</span>
          )}
        </Link>
        {renaming ? (
          <span className="run-list__rename">
            <input
              value={draft}
              data-testid="run-rename-input"
              aria-label="Run name"
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  saveRename();
                }
                if (e.key === "Escape") {
                  setRenaming(false);
                }
              }}
              spellCheck={false}
              autoComplete="off"
            />
            <button type="button" className="linkbtn" onClick={saveRename}>
              Save
            </button>
          </span>
        ) : null}
        <code className="mono muted" title={run.instanceId}>
          {shortHex(run.instanceId)}
        </code>
        {journalOnly ? (
          <span className="badge" title="Recovered from the journal (not started in this browser)">
            journal
          </span>
        ) : null}
      </div>
      <span className="muted">{new Date(run.startedAt).toLocaleTimeString()}</span>
      <span className="run-list__actions">
        {canRunAgain ? (
          <button
            type="button"
            className="linkbtn"
            data-testid="run-again"
            disabled={invoke.isPending}
            title="Re-invoke the same blueprint + args (idempotent: joins the committed result)"
            onClick={runAgain}
          >
            {invoke.isPending ? "Running…" : "Run again"}
          </button>
        ) : null}
        {run.handle ? (
          <Link
            to="/recipes"
            search={{ handle: run.handle, ...(run.args ? { args: run.args } : {}) }}
            className="linkbtn"
            data-testid="run-clone"
            title="Open this run's blueprint with its inputs prefilled — tweak and run as a new use case"
          >
            Clone
          </Link>
        ) : null}
        <Link
          to="/blueprints/new"
          search={{ clone: run.instanceId }}
          className="linkbtn"
          data-testid="run-remix"
          title="Reconstruct this run's graph in the visual builder — add agents, wire steps, run as a new workflow"
        >
          Build from this
        </Link>
        <button
          type="button"
          className="linkbtn"
          data-testid="run-rename"
          onClick={() => {
            setDraft(name ?? "");
            setRenaming((v) => !v);
          }}
          title="Rename (this browser only)"
        >
          Rename
        </button>
      </span>
      {invoke.error ? <ErrorNotice error={toUiError(invoke.error)} /> : null}
    </m.li>
  );
}
