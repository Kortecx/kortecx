import { Link, useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useEffect, useMemo, useState } from "react";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { useInvoke } from "../../kx/use-invoke";
import { useRecipeNames } from "../../kx/use-recipes";
import { useRunExport } from "../../kx/use-run-export";
import { useRuns } from "../../kx/use-runs";
import { shortHex } from "../../lib/format";
import { humanizeHandle } from "../../lib/humanize-handle";
import type { RunRecord } from "../../lib/recent-runs";
import { RUN_NAMES_CHANGED_EVENT, loadRunNames, setRunName } from "../../lib/run-names";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { Icon } from "../shell/Icon";
import { RerunDrawer } from "./RerunDrawer";

/** A run's display shape (local rename > humanized handle > short id). */
interface RunDisplay {
  readonly headline: string;
  readonly rawHandle: string | null;
  readonly customName: string | null;
  readonly journalOnly: boolean;
}

/**
 * The Runs (run-history) TABLE (PR-A): durable runs (`ListRuns`) merged with this
 * session's invocations, as aligned rows (Name · Blueprint · Started · ⋯). A
 * row-click opens the {@link RunDetailDrawer} — the "popup" with the run's
 * definition + view + the open-in-new-window action (the ONLY new-window button).
 * No fake "status" column: a run's live state needs its projection (per-run poll
 * is too costly for a list) — don't-fake-gaps.
 */
export function RunsTable() {
  const { endpoint } = useConnection();
  const { runs, clear, notWired, hasMore, loadMore, refresh } = useRuns();
  const recipeNames = useRecipeNames();
  const [filter, setFilter] = useState("");
  const [names, setNames] = useState<Record<string, string>>(() => loadRunNames(endpoint));
  const [open, setOpen] = useState<RunRecord | null>(null);
  const [rerun, setRerun] = useState<RunRecord | null>(null);

  useEffect(() => {
    setNames(loadRunNames(endpoint));
    const onChanged = (): void => setNames(loadRunNames(endpoint));
    window.addEventListener(RUN_NAMES_CHANGED_EVENT, onChanged);
    return () => window.removeEventListener(RUN_NAMES_CHANGED_EVENT, onChanged);
  }, [endpoint]);

  const fingerprintNames = recipeNames.data ?? {};

  function displayFor(r: RunRecord): RunDisplay {
    const local = names[r.instanceId];
    const handleName =
      r.handle ?? (r.recipeFingerprint ? (fingerprintNames[r.recipeFingerprint] ?? null) : null);
    const customName = local && local.trim() !== "" ? local : null;
    const journalOnly = r.handle === null && (r.args ?? null) === null;
    if (customName) {
      return { headline: customName, rawHandle: handleName, customName, journalOnly };
    }
    if (handleName) {
      return {
        headline: humanizeHandle(handleName),
        rawHandle: handleName,
        customName: null,
        journalOnly,
      };
    }
    return { headline: shortHex(r.instanceId), rawHandle: null, customName: null, journalOnly };
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
    <div data-testid="runs-tab">
      <div className="table-toolbar">
        <button type="button" className="linkbtn" onClick={refresh}>
          Refresh
        </button>
        {runs.length > 0 ? (
          <>
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
          </>
        ) : null}
      </div>
      <p className="muted">
        {notWired
          ? "This gateway does not enumerate runs — showing this session's history only."
          : "Durable runs from the journal, merged with this session's invocations. Clearing removes only this browser's history — journal runs stay."}
      </p>

      {runs.length === 0 ? (
        <EmptyState
          title="No runs yet"
          detail="Start one from the Workflows tab — pick a blueprint, fill its inputs, and run it."
        />
      ) : shown.length === 0 ? (
        <EmptyState title="No matching runs" detail="Adjust the filter above." />
      ) : (
        <table className="data-table" data-testid="run-list">
          <thead>
            <tr>
              <th scope="col">Name</th>
              <th scope="col">Blueprint</th>
              <th scope="col">Started</th>
              <th scope="col" className="data-table__actions">
                Actions
              </th>
            </tr>
          </thead>
          <tbody>
            {shown.map((r) => (
              <RunRow
                key={`${r.instanceId}:${r.terminalMoteId ?? ""}`}
                run={r}
                display={displayFor(r)}
                onOpen={() => setOpen(r)}
              />
            ))}
          </tbody>
        </table>
      )}

      {hasMore ? (
        <button type="button" className="linkbtn" data-testid="runs-load-more" onClick={loadMore}>
          Load more
        </button>
      ) : null}

      {open ? (
        <RunDetailDrawer
          run={open}
          display={displayFor(open)}
          onClose={() => setOpen(null)}
          onRerun={() => {
            setRerun(open);
            setOpen(null);
          }}
        />
      ) : null}

      {rerun ? <RerunDrawer run={rerun} onClose={() => setRerun(null)} /> : null}
    </div>
  );
}

/** One run row. The Name cell opens the detail drawer; Actions is a compact set. */
function RunRow({
  run,
  display,
  onOpen,
}: {
  run: RunRecord;
  display: RunDisplay;
  onOpen: () => void;
}) {
  return (
    <m.tr
      className="data-table__row"
      data-testid="run-row"
      whileHover={{ backgroundColor: "var(--surface-elev)" }}
    >
      <td>
        <button
          type="button"
          className="data-table__name"
          data-testid="run-open"
          onClick={onOpen}
          title="Open run details"
        >
          {display.headline}
        </button>
        {display.journalOnly ? (
          <span className="badge" title="Recovered from the journal (not started in this browser)">
            journal
          </span>
        ) : null}
      </td>
      <td>
        {display.rawHandle ? (
          <code className="mono muted">{display.rawHandle}</code>
        ) : (
          <code className="mono muted" title={run.instanceId}>
            {shortHex(run.instanceId)}
          </code>
        )}
      </td>
      <td className="muted">{new Date(run.startedAt).toLocaleString()}</td>
      <td className="data-table__actions">
        <button type="button" className="linkbtn" data-testid="run-open-details" onClick={onOpen}>
          View
        </button>
        <Link
          to="/workflows/$instanceId"
          params={{ instanceId: run.instanceId }}
          search={run.terminalMoteId ? { terminal: run.terminalMoteId } : {}}
          className="iconbtn"
          aria-label="Open run"
          title="Open run"
          data-testid="run-open-full"
        >
          <Icon name="chevron-right" size={15} />
        </Link>
      </td>
    </m.tr>
  );
}

/**
 * The run "popup" (PR-A): a slide-over (`.node-drawer`) with the run's identity +
 * definition link + view + the actions that used to live on the card menu —
 * Rename · Export · Export-with-results · Run-again · Clone · Build-from-this —
 * and the ONLY open-in-new-window button (point 4). Reuses the established
 * drawer pattern (D142.2 — no new D-number).
 */
function RunDetailDrawer({
  run,
  display,
  onClose,
  onRerun,
}: {
  run: RunRecord;
  display: RunDisplay;
  onClose: () => void;
  onRerun: () => void;
}) {
  const { endpoint } = useConnection();
  const navigate = useNavigate();
  const invoke = useInvoke();
  const exporter = useRunExport();
  const [draft, setDraft] = useState(display.customName ?? "");
  const canRunAgain = Boolean(run.handle && run.args);
  const richPending = exporter.pendingId === run.instanceId;

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

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

  return (
    <>
      <button
        type="button"
        className="node-drawer__scrim"
        aria-label="Close run details"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer"
        data-testid="run-detail-drawer"
        // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion; non-modal side-panel semantics via role+aria-label (the NodeDetailDrawer precedent)
        role="dialog"
        aria-label={`Run ${display.headline}`}
        initial={{ x: 24, opacity: 0 }}
        animate={{ x: 0, opacity: 1 }}
        transition={{ type: "spring", stiffness: 420, damping: 34 }}
      >
        <div className="node-drawer__head">
          <strong>{display.headline}</strong>
          <button type="button" className="linkbtn" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>

        <dl className="node-drawer__meta">
          <div>
            <dt>Blueprint</dt>
            <dd>
              <code className="mono">{display.rawHandle ?? "—"}</code>
            </dd>
          </div>
          <div>
            <dt>Run id</dt>
            <dd>
              <code className="mono" title={run.instanceId}>
                {shortHex(run.instanceId)}
              </code>
            </dd>
          </div>
          <div>
            <dt>Started</dt>
            <dd>{new Date(run.startedAt).toLocaleString()}</dd>
          </div>
        </dl>

        <div className="drawer-actions">
          <Link
            to="/workflows/$instanceId"
            params={{ instanceId: run.instanceId }}
            search={run.terminalMoteId ? { terminal: run.terminalMoteId } : {}}
            className="btnlink"
            data-testid="run-view-full"
            onClick={onClose}
          >
            View full run →
          </Link>
          {/* The ONLY open-in-new-window button lives in the popup (point 4). */}
          <a
            href={`/workflows/${run.instanceId}${run.terminalMoteId ? `?terminal=${run.terminalMoteId}` : ""}`}
            target="_blank"
            rel="noopener noreferrer"
            className="linkbtn"
            data-testid="run-open-newtab"
          >
            <Icon name="external-link" size={15} /> Open in new window
          </a>
        </div>

        <label className="drawer-field" htmlFor="run-rename-input">
          <span className="muted">Display name (this browser)</span>
          <span className="card-grid__rename">
            <input
              id="run-rename-input"
              value={draft}
              data-testid="run-rename-input"
              aria-label="Run name"
              placeholder={display.headline}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  setRunName(endpoint, run.instanceId, draft);
                }
              }}
              spellCheck={false}
              autoComplete="off"
            />
            <button
              type="button"
              className="linkbtn"
              data-testid="run-rename"
              onClick={() => setRunName(endpoint, run.instanceId, draft)}
            >
              Save
            </button>
          </span>
        </label>

        <div className="drawer-actions drawer-actions--wrap">
          <button
            type="button"
            className="btnlink"
            data-testid="run-rerun-changes"
            title="Edit this run's inputs and re-run — only the changed steps recompute"
            onClick={onRerun}
          >
            Re-run with changes
          </button>
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
              onClick={onClose}
            >
              Clone
            </Link>
          ) : null}
          <Link
            to="/blueprints/new"
            search={{ clone: run.instanceId }}
            className="linkbtn"
            data-testid="run-remix"
            onClick={onClose}
          >
            Build from this
          </Link>
          <button
            type="button"
            className="linkbtn"
            data-testid="run-export"
            onClick={() => exporter.exportLight(run, display.headline)}
          >
            Export
          </button>
          {run.terminalMoteId ? (
            <button
              type="button"
              className="linkbtn"
              data-testid="run-export-rich"
              disabled={richPending}
              title="Export the committed DAG + each step's resolved output"
              onClick={() => void exporter.exportRich(run, display.headline)}
            >
              {richPending ? "Exporting…" : "Export with results"}
            </button>
          ) : null}
        </div>

        {invoke.error ? <ErrorNotice error={toUiError(invoke.error)} /> : null}
        {exporter.error ? <ErrorNotice error={toUiError(exporter.error)} /> : null}
      </m.aside>
    </>
  );
}
