import type { AppSummary } from "@kortecx/sdk/web";
import { useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { type ChangeEvent, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useApps, useCloneApp, useExportAppBundle, useImportApp } from "../../kx/use-apps";
import { useHostedAppStatus, useHostedRun } from "../../kx/use-hosted-app";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { AppRunDrawer } from "../apps/AppRunDrawer";
import { AppViewPopover } from "../apps/AppViewPopover";
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
/** The two Apps sections (D213): SCHEDULED = functional automation apps (run on a
 *  trigger, pluggable into workflows); HOSTED = experience apps (a real web app the
 *  runtime serves on a local port). Cross-App HITL approvals moved to the navbar bell. */
export type AppsSectionKind = "scheduled" | "hosted";

const APPS_SECTIONS: ReadonlyArray<{ id: AppsSectionKind; label: string; hint: string }> = [
  {
    id: "scheduled",
    label: "Scheduled",
    hint: "Automation apps — run on a trigger / in workflows",
  },
  { id: "hosted", label: "Hosted", hint: "Web apps the runtime scaffolds and serves on a port" },
];

/** Route an App to its section by the backend lane (defaults to scheduled/functional). */
function sectionOf(app: AppSummary): AppsSectionKind {
  return app.kind === "experience" ? "hosted" : "scheduled";
}

export function AppsSection({
  section = "scheduled",
  onSection,
  view = "box",
  onView,
}: {
  section?: AppsSectionKind;
  onSection?: (section: AppsSectionKind) => void;
  /** The catalog layout — box/card grid (default) or a scannable table. */
  view?: "box" | "table";
  onView?: (view: "box" | "table") => void;
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
  const sectioned = apps.filter((a) => sectionOf(a) === section);
  const hosted = section === "hosted";

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

      <fieldset className="view-toggle" aria-label="Apps section" data-testid="apps-sections">
        {APPS_SECTIONS.map((s) => (
          <button
            key={s.id}
            type="button"
            data-testid={`apps-section-${s.id}`}
            aria-pressed={section === s.id}
            title={s.hint}
            onClick={() => onSection?.(s.id)}
          >
            {s.label}
          </button>
        ))}
      </fieldset>

      {notice ? (
        <output className="muted" data-testid="apps-notice">
          {notice}
        </output>
      ) : null}
      {importApp.error ? (
        <ErrorNotice error={toUiError(importApp.error)} onRetry={() => importApp.reset()} />
      ) : null}
      {exportBundle.error ? (
        <ErrorNotice error={toUiError(exportBundle.error)} onRetry={() => exportBundle.reset()} />
      ) : null}

      {creating ? <NewAppForm onClose={() => setCreating(false)} initialKind={section} /> : null}

      {isLoading ? <EmptyState title="Loading apps…" /> : null}

      {notWired ? (
        <EmptyState
          title="Apps not available"
          detail="This gateway does not expose the App catalog (an older build, or the apps.db sidecar is absent)."
        />
      ) : isError ? (
        <ErrorNotice error={toUiError(error)} onRetry={() => void refetch()} />
      ) : !isLoading && sectioned.length === 0 ? (
        <EmptyState
          title={hosted ? "No hosted apps yet" : "No scheduled apps yet"}
          detail={
            hosted
              ? "Create a hosted web app with New App — the runtime scaffolds a React / Next.js project and serves it on a local port."
              : "Author an automation app with `kx app save <file>`, the `app()` SDK builder, or New App — then run it on a trigger or in a workflow."
          }
        />
      ) : null}

      {sectioned.length > 0 ? (
        <>
          <div className="apps-section__panel-head">
            <span className="muted">
              {sectioned.length} {section} app{sectioned.length === 1 ? "" : "s"}
            </span>
            <fieldset
              className="view-toggle view-toggle--compact view-toggle--icons"
              aria-label="Apps layout"
              data-testid="apps-view-toggle"
            >
              <button
                type="button"
                data-testid="apps-view-box"
                aria-pressed={view === "box"}
                title="Card view"
                onClick={() => onView?.("box")}
              >
                <Icon name="grid" size={15} />
              </button>
              <button
                type="button"
                data-testid="apps-view-table"
                aria-pressed={view === "table"}
                title="Table view"
                onClick={() => onView?.("table")}
              >
                <Icon name="table" size={15} />
              </button>
            </fieldset>
          </div>
          {view === "box" ? (
            <m.div
              className="card-grid"
              data-testid="apps-catalog"
              data-view="box"
              variants={stagger()}
              initial="hidden"
              animate="show"
            >
              {sectioned.map((a) => (
                <AppCard
                  key={a.handle}
                  app={a}
                  hosted={hosted}
                  downloadPending={exportBundle.isPending}
                  onRun={run}
                  onView={setSummaryFor}
                  onOpen={(handle) => void navigate({ to: "/apps/$handle", params: { handle } })}
                  onDownload={download}
                  onDuplicate={setDuplicating}
                />
              ))}
            </m.div>
          ) : (
            <AppsTable
              apps={sectioned}
              hosted={hosted}
              downloadPending={exportBundle.isPending}
              onRun={run}
              onView={setSummaryFor}
              onOpen={(handle) => void navigate({ to: "/apps/$handle", params: { handle } })}
              onDownload={download}
              onDuplicate={setDuplicating}
            />
          )}
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
      {runHandle ? <AppRunDrawer handle={runHandle} onClose={() => setRunHandle(null)} /> : null}
    </section>
  );
}

/** One App in the catalog — name + version with a top-right action cluster (▶ Run,
 *  lock state, honest-disabled Share, download, and a kebab for view/open/duplicate),
 *  then the description, step/tag chips, and the raw handle. */
function AppCard({
  app,
  hosted,
  downloadPending,
  onRun,
  onView,
  onOpen,
  onDownload,
  onDuplicate,
}: {
  app: AppSummary;
  hosted: boolean;
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
          className="card-grid__title card-grid__title-btn card-grid__title--clamp"
          title={`${app.name} — view details`}
          data-testid={`app-card-view-${app.handle}`}
          onClick={() => onView(app.handle)}
        >
          {app.name}
        </button>
        <span className="chip chip--tag">v{app.version}</span>
        <div className="card-grid__head-actions">
          {hosted ? (
            // Hosted cards carry only the live status pill + Run-in-tab; lock/share/
            // download don't apply to a served web app (a hosted app isn't lockable and
            // its .kxapp bundle omits the project tree today), so they'd only crowd the
            // head. Download stays reachable via the kebab.
            <HostedControls handle={app.handle} />
          ) : (
            <>
              <button
                type="button"
                className="iconbtn"
                data-testid={`app-run-${app.handle}`}
                title="Run this App"
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
            </>
          )}
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
                  {/* A distinct glyph from Run: Open browses the App's file tree / IDE,
                      it does not launch anything (the hosted Run keeps external-link). */}
                  <Icon name="artifacts" size={15} />
                  <span>Open project</span>
                </button>
                {hosted ? (
                  <button
                    type="button"
                    role="menuitem"
                    className="popover__item"
                    data-testid={`app-download-${app.handle}`}
                    disabled={downloadPending}
                    onClick={() => {
                      close();
                      onDownload(app.handle);
                    }}
                  >
                    <Icon name="download" size={15} />
                    <span>Download bundle</span>
                  </button>
                ) : null}
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
      {hosted ? <HostedDetail handle={app.handle} /> : null}

      <div className="card-grid__tags">
        {hosted ? (
          // A hosted app has no blueprint steps — show what it IS, not a misleading "0 steps".
          <span className="chip chip--tag">web app</span>
        ) : (
          <span className="chip chip--tag">
            {app.stepCount} step{app.stepCount === 1 ? "" : "s"}
          </span>
        )}
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

/** D213 — a hosted app's live status pill (running / starting / stopped / failed). */
function HostedStatusPill({ handle }: { handle: string }) {
  const { status } = useHostedAppStatus(handle, true);
  const state = status?.state ?? "stopped";
  const tone =
    state === "running"
      ? "committed"
      : state === "failed"
        ? "failed"
        : state === "stopped"
          ? "unknown"
          : "pending";
  return (
    <span
      className={`pill pill--${tone}`}
      data-testid={`hosted-status-${handle}`}
      title={status?.detail || state}
    >
      {state}
    </span>
  );
}

/** D213 — the hosted-app supervisor detail line (install / dev-server progress or the
 *  failure reason). The minimal "surface the supervisor logs" affordance: it shows the
 *  live `HostedAppStatus.detail` while the app is materializing/installing/starting or has
 *  failed, and stays quiet once it is plainly running or stopped. React Query dedups the
 *  poll with the status pill, so this adds no extra network. */
function HostedDetail({ handle }: { handle: string }) {
  const { status } = useHostedAppStatus(handle, true);
  const detail = status?.detail?.trim();
  const state = status?.state ?? "stopped";
  if (!detail || state === "stopped" || state === "running") {
    return null;
  }
  return (
    <p className="card-grid__sub muted" data-testid={`hosted-detail-${handle}`}>
      {detail}
    </p>
  );
}

/** D213 — the hosted-app Run control: start the dev server, open it once actually running,
 *  surface start errors, and honest-disable when the gateway lacks the hosted-apps feature. */
function HostedRunButton({ handle }: { handle: string }) {
  const { run, disabled, busy, error } = useHostedRun(handle);
  if (disabled) {
    return (
      <span
        className="iconbtn iconbtn--disabled"
        aria-disabled="true"
        title="Hosted apps aren't available on this gateway (serve with the hosted-apps feature)"
        data-testid={`hosted-run-${handle}`}
      >
        <Icon name="external-link" size={16} />
      </span>
    );
  }
  return (
    <>
      <button
        type="button"
        className="iconbtn"
        data-testid={`hosted-run-${handle}`}
        disabled={busy}
        aria-busy={busy}
        title={busy ? "Starting the hosted app…" : "Run this hosted app (opens in a new tab)"}
        aria-label="Run hosted app"
        onClick={run}
      >
        <Icon name="external-link" size={16} />
      </button>
      {error ? (
        <span
          className="hosted-run__error field-error"
          role="alert"
          data-testid={`hosted-run-error-${handle}`}
        >
          {error}
        </span>
      ) : null}
    </>
  );
}

/** The hosted-app card cluster: a status pill + the Run-in-new-tab control. */
function HostedControls({ handle }: { handle: string }) {
  return (
    <>
      <HostedStatusPill handle={handle} />
      <HostedRunButton handle={handle} />
    </>
  );
}

/** The Apps TABLE layout — a scannable alternative to the card grid. Hosted rows carry a
 *  live status column; every row shares the run/open/download/duplicate action cluster. */
function AppsTable({
  apps,
  hosted,
  downloadPending,
  onRun,
  onView,
  onOpen,
  onDownload,
  onDuplicate,
}: {
  apps: AppSummary[];
  hosted: boolean;
  downloadPending: boolean;
  onRun: (handle: string) => void;
  onView: (handle: string) => void;
  onOpen: (handle: string) => void;
  onDownload: (handle: string) => void;
  onDuplicate: (handle: string) => void;
}) {
  return (
    <table className="trail-table apps-table" data-testid="apps-table">
      <thead>
        <tr>
          <th>Name</th>
          <th>Version</th>
          <th>Steps</th>
          <th>Tags</th>
          {hosted ? <th>Status</th> : null}
          <th>Actions</th>
        </tr>
      </thead>
      <tbody>
        {apps.map((a) => (
          <tr key={a.handle} data-testid={`app-row-${a.handle}`}>
            <td>
              <button
                type="button"
                className="linkbtn"
                data-testid={`app-card-view-${a.handle}`}
                onClick={() => onView(a.handle)}
              >
                {a.name}
              </button>
              <div>
                <code className="mono muted mono-trunc" title={a.handle}>
                  {a.handle}
                </code>
              </div>
            </td>
            <td>v{a.version}</td>
            <td>{hosted ? "—" : a.stepCount}</td>
            <td>{a.tags.join(", ") || "—"}</td>
            {hosted ? (
              <td>
                <HostedStatusPill handle={a.handle} />
              </td>
            ) : null}
            <td className="app-row__actions">
              {hosted ? (
                <HostedRunButton handle={a.handle} />
              ) : (
                <button
                  type="button"
                  className="iconbtn"
                  data-testid={`app-run-${a.handle}`}
                  title="Run this App"
                  aria-label="Run"
                  onClick={() => onRun(a.handle)}
                >
                  <Icon name="play" size={16} />
                </button>
              )}
              <button
                type="button"
                className="iconbtn"
                data-testid={`app-open-${a.handle}`}
                title="Open project"
                aria-label="Open project"
                onClick={() => onOpen(a.handle)}
              >
                <Icon name="artifacts" size={15} />
              </button>
              <button
                type="button"
                className="iconbtn"
                data-testid={`app-download-${a.handle}`}
                disabled={downloadPending}
                title="Download a portable .kxapp bundle"
                aria-label="Download bundle"
                onClick={() => onDownload(a.handle)}
              >
                <Icon name="download" size={16} />
              </button>
              <button
                type="button"
                className="iconbtn"
                data-testid={`app-duplicate-${a.handle}`}
                title="Duplicate locally"
                aria-label="Duplicate"
                onClick={() => onDuplicate(a.handle)}
              >
                <Icon name="copy" size={15} />
              </button>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
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
  return createPortal(
    <>
      <button
        type="button"
        className="node-drawer__scrim node-drawer__scrim--overlay"
        aria-label="Cancel duplicate"
        onClick={onClose}
      />
      <div className="dialog-center dialog-center--overlay">
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
    </>,
    document.body,
  );
}
