import type { AppSummary } from "@kortecx/sdk/web";
import { useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { type ChangeEvent, useEffect, useRef, useState } from "react";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useApps, useCloneApp, useExportAppBundle, useImportApp } from "../../kx/use-apps";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { AppRunDrawer } from "../apps/AppRunDrawer";
import { AppViewPopover } from "../apps/AppViewPopover";
import { ApprovalsInbox } from "../apps/ApprovalsInbox";
import { Icon } from "../shell/Icon";
import { Popover } from "../shell/Popover";
import { NewAppForm } from "./NewAppForm";

/**
 * The Apps catalog (POC-4) — a READ-ONLY view over the caller's durable
 * `kortecx.app/v1` envelopes (`ListApps`). Each App is a portable blueprint
 * wrapped with by-reference references + a 4-axis steering config; Run compiles
 * its blueprint and submits it (the server re-resolves every warrant from the
 * caller's grants, SN-8). "View details" opens a read-only summary popover.
 *
 * POC-5a adds the agentic "New App" scaffold (inline panel) — author an App
 * here, the agent scaffolds a starter project tree into its CoW branch, then Open
 * browses + edits it (POC-5d). The SDK/CLI surface (`kx app` / `app()`) still
 * authors too. Share + Schedule are Cloud (D129) — honest-disabled chips, never
 * fake controls. Nothing currently exposed disappears: Apps is ADDITIVE alongside
 * Workflows/Blueprints (the consolidation rides the deferred redesign).
 */
/** The Apps section views: the App catalog (default) and the cross-App HITL
 *  approvals inbox (a single pending queue across every App). */
export type AppsTab = "catalog" | "approvals";

const APPS_TABS: ReadonlyArray<{ id: AppsTab; label: string }> = [
  { id: "catalog", label: "Catalog" },
  { id: "approvals", label: "Approvals" },
];

export function AppsSection({
  tab = "catalog",
  onTab,
  view = "box",
  onView,
}: {
  tab?: AppsTab;
  onTab?: (tab: AppsTab) => void;
  /** The catalog layout — box/card grid (default) or compact list rows. */
  view?: "list" | "box";
  onView?: (view: "list" | "box") => void;
} = {}) {
  const navigate = useNavigate();
  const { apps, notWired, isLoading, isError, error, refetch } = useApps();
  const exportBundle = useExportAppBundle();
  const importApp = useImportApp();
  const cloneApp = useCloneApp();
  const [summaryFor, setSummaryFor] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [duplicating, setDuplicating] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [runHandle, setRunHandle] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Open the typed run drawer for this App — it reads the App's `input_schema` (via
  // GetApp) and submits with the collected args (an App with no inputs runs in one
  // click, then routes to the live run). This replaces a direct argless
  // `runApp.mutate({ handle })`, which silently ran any App declaring inputs with an
  // empty prompt — a wrong run for every parameterized App.
  function run(handle: string): void {
    setRunHandle(handle);
  }

  // Download: export the App as a portable .kxapp bundle (envelope + content closure)
  // and stream it to the browser as a file — no server round-trip beyond the export.
  function download(handle: string): void {
    setNotice(null);
    exportBundle.mutate(
      { handle },
      {
        onSuccess: (wire) => {
          const url = URL.createObjectURL(new Blob([wire], { type: "application/json" }));
          const a = document.createElement("a");
          a.href = url;
          a.download = `${handle.replace(/\//g, "-")}.kxapp`;
          a.click();
          URL.revokeObjectURL(url);
          setNotice(`Downloaded ${handle}`);
        },
      },
    );
  }

  async function onImportFile(e: ChangeEvent<HTMLInputElement>): Promise<void> {
    const file = e.target.files?.[0];
    e.target.value = ""; // allow re-selecting the same file
    if (!file) {
      return;
    }
    setNotice(null);
    const text = await file.text();
    importApp.mutate(
      { bundle: text, force: false },
      { onSuccess: ({ handle }) => setNotice(`Imported ${handle}`) },
    );
  }

  function duplicate(newname: string): void {
    if (duplicating === null) {
      return;
    }
    cloneApp.mutate(
      { handle: duplicating, newname },
      {
        onSuccess: ({ handle }) => {
          setNotice(`Duplicated to ${handle}`);
          setDuplicating(null);
        },
      },
    );
  }

  return (
    <section className="screen" data-testid="apps-section">
      <div className="section-head">
        <div>
          <h1>Apps</h1>
          <p className="muted">
            Durable, reusable Apps — create one here, or open an App to run and edit it.
          </p>
        </div>
        <div className="section-head__actions">
          <input
            ref={fileInputRef}
            type="file"
            accept=".kxapp,application/json"
            style={{ display: "none" }}
            data-testid="app-import-input"
            onChange={(e) => void onImportFile(e)}
          />
          <button
            type="button"
            className="btn-ghost"
            data-testid="import-app"
            disabled={importApp.isPending}
            title="Import a portable .kxapp bundle under your own account"
            onClick={() => fileInputRef.current?.click()}
          >
            <Icon name="download" size={15} />
            <span>{importApp.isPending ? "Importing…" : "Import"}</span>
          </button>
          <button
            type="button"
            className="btn-primary"
            data-testid="new-app"
            aria-expanded={creating}
            onClick={() => setCreating((c) => !c)}
          >
            {creating ? "Close" : "New App"}
          </button>
        </div>
      </div>

      <fieldset className="view-toggle" aria-label="Apps view" data-testid="apps-tabs">
        {APPS_TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            data-testid={`apps-tab-${t.id}`}
            aria-pressed={tab === t.id}
            onClick={() => onTab?.(t.id)}
          >
            {t.label}
          </button>
        ))}
      </fieldset>

      {tab === "approvals" ? (
        <ApprovalsInbox />
      ) : (
        <>
          {notice ? (
            <output className="muted" data-testid="apps-notice">
              {notice}
            </output>
          ) : null}
          {importApp.error ? (
            <ErrorNotice error={toUiError(importApp.error)} onRetry={() => importApp.reset()} />
          ) : null}
          {exportBundle.error ? (
            <ErrorNotice
              error={toUiError(exportBundle.error)}
              onRetry={() => exportBundle.reset()}
            />
          ) : null}

          {creating ? <NewAppForm onClose={() => setCreating(false)} /> : null}

          {isLoading ? <EmptyState title="Loading apps…" /> : null}

          {notWired ? (
            <EmptyState
              title="Apps not available"
              detail="This gateway does not expose the App catalog (an older build, or the apps.db sidecar is absent)."
            />
          ) : isError ? (
            <ErrorNotice error={toUiError(error)} onRetry={() => void refetch()} />
          ) : !isLoading && apps.length === 0 ? (
            <EmptyState
              title="No apps yet"
              detail="Author an App with `kx app save <file>` or the `app()` SDK builder, then it appears here to inspect and run."
            />
          ) : null}

          {apps.length > 0 ? (
            <>
              <fieldset
                className="view-toggle view-toggle--compact"
                aria-label="Catalog layout"
                data-testid="apps-view-toggle"
              >
                <button
                  type="button"
                  data-testid="apps-view-box"
                  aria-pressed={view === "box"}
                  onClick={() => onView?.("box")}
                >
                  Box
                </button>
                <button
                  type="button"
                  data-testid="apps-view-list"
                  aria-pressed={view === "list"}
                  onClick={() => onView?.("list")}
                >
                  List
                </button>
              </fieldset>
              <m.div
                className={`card-grid${view === "list" ? " card-grid--list" : ""}`}
                data-testid="apps-catalog"
                data-view={view}
                variants={stagger()}
                initial="hidden"
                animate="show"
              >
                {apps.map((a) => (
                  <AppCard
                    key={a.handle}
                    app={a}
                    pending={false}
                    downloadPending={exportBundle.isPending}
                    onRun={run}
                    onView={setSummaryFor}
                    onOpen={(handle) => void navigate({ to: "/apps/$handle", params: { handle } })}
                    onDownload={download}
                    onDuplicate={setDuplicating}
                  />
                ))}
              </m.div>
            </>
          ) : null}

          {cloneApp.error ? (
            <ErrorNotice error={toUiError(cloneApp.error)} onRetry={() => cloneApp.reset()} />
          ) : null}

          {summaryFor ? (
            <AppViewPopover handle={summaryFor} onClose={() => setSummaryFor(null)} />
          ) : null}
          {duplicating ? (
            <DuplicateDialog
              handle={duplicating}
              pending={cloneApp.isPending}
              onSubmit={duplicate}
              onClose={() => {
                setDuplicating(null);
                cloneApp.reset();
              }}
            />
          ) : null}
          {runHandle ? (
            <AppRunDrawer handle={runHandle} onClose={() => setRunHandle(null)} />
          ) : null}
        </>
      )}
    </section>
  );
}

/** One App in the catalog — name + version with a top-right action cluster (▶ Run,
 *  lock state, honest-disabled Share, download, and a kebab for view/open/duplicate),
 *  then the description, step/tag chips, and the raw handle. */
function AppCard({
  app,
  pending,
  downloadPending,
  onRun,
  onView,
  onOpen,
  onDownload,
  onDuplicate,
}: {
  app: AppSummary;
  pending: boolean;
  downloadPending: boolean;
  onRun: (handle: string) => void;
  onView: (handle: string) => void;
  onOpen: (handle: string) => void;
  onDownload: (handle: string) => void;
  onDuplicate: (handle: string) => void;
}) {
  return (
    <m.article
      variants={fadeUp}
      {...hoverLift}
      className="glow-card glow-card--hover card-grid__card"
      data-testid={`app-card-${app.handle}`}
    >
      <div className="card-grid__head">
        <button
          type="button"
          className="card-grid__title card-grid__title-btn"
          title={`${app.name} — view details`}
          data-testid={`app-card-view-${app.handle}`}
          onClick={() => onView(app.handle)}
        >
          {app.name}
        </button>
        <span className="chip chip--tag">v{app.version}</span>
        <div className="card-grid__head-actions">
          <button
            type="button"
            className="iconbtn"
            data-testid={`app-run-${app.handle}`}
            disabled={pending}
            title={pending ? "Running…" : "Run this App"}
            aria-label="Run"
            onClick={() => onRun(app.handle)}
          >
            <Icon name="play" size={16} />
          </button>
          <span
            className="iconbtn iconbtn--static"
            data-testid={`app-lock-${app.handle}`}
            data-locked={app.locked ? "true" : "false"}
            title={
              app.locked
                ? "Locked — agentic in-CAS edits are refused (manage from the App page)"
                : "Unlocked"
            }
          >
            <Icon name={app.locked ? "lock" : "unlock"} size={16} />
          </span>
          <span
            className="iconbtn iconbtn--disabled"
            aria-disabled="true"
            title="Sharing across parties is a Cloud capability"
          >
            <Icon name="share" size={16} />
          </span>
          <button
            type="button"
            className="iconbtn"
            data-testid={`app-download-${app.handle}`}
            disabled={downloadPending}
            title="Download a portable .kxapp bundle (envelope + content closure)"
            aria-label="Download bundle"
            onClick={() => onDownload(app.handle)}
          >
            <Icon name="download" size={16} />
          </button>
          <Popover
            trigger={<Icon name="menu" size={16} />}
            triggerClassName="iconbtn"
            triggerLabel="App actions"
            triggerTestId={`app-menu-${app.handle}`}
            align="right"
            direction="down"
            menuTestId={`app-menu-panel-${app.handle}`}
          >
            {(close) => (
              <>
                <button
                  type="button"
                  role="menuitem"
                  className="popover__item"
                  data-testid={`app-view-${app.handle}`}
                  onClick={() => {
                    close();
                    onView(app.handle);
                  }}
                >
                  <Icon name="recipes" size={15} />
                  <span>View details</span>
                </button>
                <button
                  type="button"
                  role="menuitem"
                  className="popover__item"
                  data-testid={`app-open-${app.handle}`}
                  onClick={() => {
                    close();
                    onOpen(app.handle);
                  }}
                >
                  <Icon name="external-link" size={15} />
                  <span>Open project</span>
                </button>
                <button
                  type="button"
                  role="menuitem"
                  className="popover__item"
                  data-testid={`app-duplicate-${app.handle}`}
                  title="Duplicate this App locally under a new name"
                  onClick={() => {
                    close();
                    onDuplicate(app.handle);
                  }}
                >
                  <Icon name="copy" size={15} />
                  <span>Duplicate</span>
                </button>
              </>
            )}
          </Popover>
        </div>
      </div>

      {app.description ? <p className="card-grid__sub">{app.description}</p> : null}

      <div className="card-grid__tags">
        <span className="chip chip--tag">
          {app.stepCount} step{app.stepCount === 1 ? "" : "s"}
        </span>
        {app.tags.map((t) => (
          <span key={t} className="chip chip--tag">
            {t}
          </span>
        ))}
      </div>

      <code className="mono card-grid__handle" title={app.handle}>
        {app.handle}
      </code>
    </m.article>
  );
}

/** A compact dialog to name a local duplicate (clone) of an App. */
function DuplicateDialog({
  handle,
  pending,
  onSubmit,
  onClose,
}: {
  handle: string;
  pending: boolean;
  onSubmit: (newname: string) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    inputRef.current?.focus(); // focus the sole input (opened on an explicit action)
    function onKey(e: KeyboardEvent): void {
      if (e.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  const trimmed = name.trim();
  return (
    <>
      <button
        type="button"
        className="node-drawer__scrim"
        aria-label="Cancel duplicate"
        onClick={onClose}
      />
      <div className="dialog-center">
        <m.div
          className="dialog-card"
          data-testid="app-duplicate-dialog"
          // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion; modal semantics via role+aria-label (the AppViewPopover precedent)
          role="dialog"
          aria-label={`Duplicate ${handle}`}
          initial={{ y: 12, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          transition={{ type: "spring", stiffness: 420, damping: 34 }}
        >
          <h2 className="dialog-card__title">Duplicate App</h2>
          <p className="muted">
            A local frozen copy of <code className="mono">{handle}</code> under a new name (content
            is already resident — no transfer; the copy records its lineage).
          </p>
          <label className="dialog-card__label" htmlFor="dup-name">
            New name
          </label>
          <input
            ref={inputRef}
            id="dup-name"
            className="input"
            data-testid="app-duplicate-name"
            value={name}
            placeholder="My App copy"
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && trimmed !== "") {
                onSubmit(trimmed);
              }
            }}
          />
          <div className="dialog-card__actions">
            <button type="button" className="btn-ghost" onClick={onClose}>
              Cancel
            </button>
            <button
              type="button"
              className="btn-primary"
              data-testid="app-duplicate-submit"
              disabled={pending || trimmed === ""}
              onClick={() => onSubmit(trimmed)}
            >
              {pending ? "Duplicating…" : "Duplicate"}
            </button>
          </div>
        </m.div>
      </div>
    </>
  );
}
