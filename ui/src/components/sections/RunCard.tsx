import { Link, useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useState } from "react";
import { fadeUp, hoverLift } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { useInvoke } from "../../kx/use-invoke";
import { useRunExport } from "../../kx/use-run-export";
import { shortHex } from "../../lib/format";
import type { RunRecord } from "../../lib/recent-runs";
import { setRunName } from "../../lib/run-names";
import { ErrorNotice } from "../ErrorNotice";
import { Icon } from "../shell/Icon";
import { Popover } from "../shell/Popover";

/**
 * One run as a bordered card (PR-4.1b): a clean display name headline (the raw
 * handle stays a secondary mono chip) + a per-card action menu (the reused
 * `Popover`/D148). Sharing + scheduling are cross-party / cloud (D129) — shown
 * as honest-disabled "Cloud" chips (D142 don't-fake-gaps); a committed run is
 * immutable, so there is deliberately NO "settings" item (Clone / Build-from
 * are the real re-config paths). Open-in-new-tab uses `rel="noopener"`.
 */
export function RunCard({
  run,
  headline,
  rawHandle,
  customName,
}: {
  run: RunRecord;
  /** The display headline (local rename > humanized handle > short id). */
  headline: string;
  /** The raw handle for the secondary mono chip + the e2e/disambiguation. */
  rawHandle: string | null;
  /** The current client-local custom name (seeds the rename draft), or null. */
  customName: string | null;
}) {
  const { endpoint } = useConnection();
  const navigate = useNavigate();
  const invoke = useInvoke();
  const exporter = useRunExport();
  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState(customName ?? "");

  // A durable-only card (recovered from the journal, not started in this browser).
  const journalOnly = run.handle === null && (run.args ?? null) === null;
  const canRunAgain = Boolean(run.handle && run.args);
  const richPending = exporter.pendingId === run.instanceId;

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
    <m.article
      className="glow-card glow-card--hover card-grid__card"
      data-testid="run-card"
      variants={fadeUp}
      {...hoverLift}
    >
      <div className="card-grid__head">
        <Link
          to="/workflows/$instanceId"
          params={{ instanceId: run.instanceId }}
          search={run.terminalMoteId ? { terminal: run.terminalMoteId } : {}}
          className="card-grid__title"
          data-testid="run-open"
        >
          {headline}
        </Link>
        <Popover
          trigger={<Icon name="menu" size={16} />}
          triggerClassName="iconbtn"
          triggerLabel="Run actions"
          triggerTestId="run-menu"
          align="right"
          direction="down"
          menuTestId="run-menu-panel"
        >
          {(close) => (
            <>
              <Link
                to="/workflows/$instanceId"
                params={{ instanceId: run.instanceId }}
                search={run.terminalMoteId ? { terminal: run.terminalMoteId } : {}}
                target="_blank"
                rel="noopener noreferrer"
                role="menuitem"
                className="popover__item"
                data-testid="run-open-newtab"
                onClick={close}
              >
                <Icon name="external-link" size={15} />
                <span>Open in new tab</span>
              </Link>
              {canRunAgain ? (
                <button
                  type="button"
                  role="menuitem"
                  className="popover__item"
                  data-testid="run-again"
                  disabled={invoke.isPending}
                  title="Re-invoke the same blueprint + args (idempotent: joins the committed result)"
                  onClick={() => {
                    close();
                    runAgain();
                  }}
                >
                  <Icon name="refresh" size={15} />
                  <span>{invoke.isPending ? "Running…" : "Run again"}</span>
                </button>
              ) : null}
              {run.handle ? (
                <Link
                  to="/recipes"
                  search={{ handle: run.handle, ...(run.args ? { args: run.args } : {}) }}
                  role="menuitem"
                  className="popover__item"
                  data-testid="run-clone"
                  title="Open this run's blueprint with its inputs prefilled — tweak and run as a new use case"
                  onClick={close}
                >
                  <Icon name="copy" size={15} />
                  <span>Clone</span>
                </Link>
              ) : null}
              <Link
                to="/blueprints/new"
                search={{ clone: run.instanceId }}
                role="menuitem"
                className="popover__item"
                data-testid="run-remix"
                title="Reconstruct this run's graph in the visual builder"
                onClick={close}
              >
                <Icon name="recipes" size={15} />
                <span>Build from this</span>
              </Link>
              <button
                type="button"
                role="menuitem"
                className="popover__item"
                data-testid="run-rename"
                onClick={() => {
                  setDraft(customName ?? "");
                  setRenaming(true);
                  close();
                }}
              >
                <Icon name="settings" size={15} />
                <span>Rename</span>
              </button>
              <button
                type="button"
                role="menuitem"
                className="popover__item"
                data-testid="run-export"
                title="Export this run's record as JSON (this browser)"
                onClick={() => {
                  close();
                  exporter.exportLight(run, headline);
                }}
              >
                <Icon name="download" size={15} />
                <span>Export</span>
              </button>
              {run.terminalMoteId ? (
                <button
                  type="button"
                  role="menuitem"
                  className="popover__item"
                  data-testid="run-export-rich"
                  disabled={richPending}
                  title="Export the committed DAG + each step's resolved output (fetches from the gateway)"
                  onClick={() => {
                    void exporter.exportRich(run, headline).then(close);
                  }}
                >
                  <Icon name="download" size={15} />
                  <span>{richPending ? "Exporting…" : "Export with results"}</span>
                </button>
              ) : null}
              <button
                type="button"
                role="menuitem"
                className="popover__item popover__item--disabled"
                data-testid="run-share"
                disabled
                aria-disabled="true"
                title="Share across parties — a managed cloud capability"
              >
                <Icon name="share" size={15} />
                <span>Share</span>
                <span className="chip chip--soon">Cloud</span>
              </button>
              <button
                type="button"
                role="menuitem"
                className="popover__item popover__item--disabled"
                data-testid="run-schedule"
                disabled
                aria-disabled="true"
                title="Schedule recurring runs — a managed cloud capability"
              >
                <Icon name="calendar" size={15} />
                <span>Schedule</span>
                <span className="chip chip--soon">Cloud</span>
              </button>
            </>
          )}
        </Popover>
      </div>

      {renaming ? (
        <span className="card-grid__rename">
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

      <div className="card-grid__meta">
        {rawHandle ? (
          <code className="mono card-grid__handle" title={rawHandle}>
            {rawHandle}
          </code>
        ) : (
          <code className="mono card-grid__handle" title={run.instanceId}>
            {shortHex(run.instanceId)}
          </code>
        )}
        {journalOnly ? (
          <span className="badge" title="Recovered from the journal (not started in this browser)">
            journal
          </span>
        ) : null}
        <span className="card-grid__time">{new Date(run.startedAt).toLocaleTimeString()}</span>
      </div>

      {invoke.error ? <ErrorNotice error={toUiError(invoke.error)} /> : null}
      {exporter.error ? <ErrorNotice error={toUiError(exporter.error)} /> : null}
    </m.article>
  );
}
