import type { AppSummary } from "@kortecx/sdk/web";
import { useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { type ChangeEvent, useEffect, useRef, useState } from "react";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import {
  useApp,
  useApps,
  useCloneApp,
  useExportAppBundle,
  useImportApp,
  useRunApp,
} from "../../kx/use-apps";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { AppViewPopover } from "../apps/AppViewPopover";
import { ApprovalsInbox } from "../apps/ApprovalsInbox";
import { CodeViewer } from "../editor/CodeViewer";
import { Icon } from "../shell/Icon";
import { Popover } from "../shell/Popover";
import { NewAppForm } from "./NewAppForm";

/**
 * The Apps catalog (POC-4) — a READ-ONLY view over the caller's durable
 * `kortecx.app/v1` envelopes (`ListApps`). Each App is a portable blueprint
 * wrapped with by-reference references + a 4-axis steering config; Run compiles
 * its blueprint and submits it (the server re-resolves every warrant from the
 * caller's grants, SN-8). Inspect shows the full envelope (Monaco, read-only).
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
}: {
  tab?: AppsTab;
  onTab?: (tab: AppsTab) => void;
} = {}) {
  const navigate = useNavigate();
  const { apps, notWired, isLoading, isError, error, refetch } = useApps();
  const runApp = useRunApp();
  const exportBundle = useExportAppBundle();
  const importApp = useImportApp();
  const cloneApp = useCloneApp();
  const [viewing, setViewing] = useState<string | null>(null);
  const [summaryFor, setSummaryFor] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [duplicating, setDuplicating] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  function run(handle: string): void {
    runApp.mutate(
      { handle },
      {
        onSuccess: ({ instanceId }) => {
          void navigate({ to: "/workflows/$instanceId", params: { instanceId } });
        },
      },
    );
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

  const runError = runApp.error ? toUiError(runApp.error) : null;
  // G2: a RunApp `missing integration: <name>` refusal is actionable — offer to open
  // the Connections panel to register the connection the App references.
  const missingIntegration = runError !== null && /missing integration/i.test(runError.message);
  const runAction = missingIntegration
    ? { label: "Set up integration", onClick: () => void navigate({ to: "/tools" }) }
    : undefined;

  return (
    <section className="screen" data-testid="apps-section">
      <div className="section-head">
        <div>
          <h1>Apps</h1>
          <p className="muted">
            Durable, reusable Apps — a portable blueprint plus its references, steering, and replay
            intent. Create one here (the agent scaffolds a starter project tree), or author with{" "}
            <code className="mono">kx app</code> / the <code className="mono">app()</code> SDK. Open
            an App to browse and edit its project files.
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
            <m.div
              className="card-grid"
              data-testid="apps-catalog"
              variants={stagger()}
              initial="hidden"
              animate="show"
            >
              {apps.map((a) => (
                <AppCard
                  key={a.handle}
                  app={a}
                  pending={runApp.isPending}
                  downloadPending={exportBundle.isPending}
                  onRun={run}
                  onView={setSummaryFor}
                  onInspect={setViewing}
                  onOpen={(handle) => void navigate({ to: "/apps/$handle", params: { handle } })}
                  onDownload={download}
                  onDuplicate={setDuplicating}
                />
              ))}
            </m.div>
          ) : null}

          {runError ? (
            <ErrorNotice error={runError} onRetry={() => runApp.reset()} action={runAction} />
          ) : null}

          {cloneApp.error ? (
            <ErrorNotice error={toUiError(cloneApp.error)} onRetry={() => cloneApp.reset()} />
          ) : null}

          {summaryFor ? (
            <AppViewPopover handle={summaryFor} onClose={() => setSummaryFor(null)} />
          ) : null}
          {viewing ? <AppDetailDrawer handle={viewing} onClose={() => setViewing(null)} /> : null}
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
        </>
      )}
    </section>
  );
}

/** One App in the catalog — name, description, version/step/tag chips, the raw
 *  handle, and Run / Inspect actions (+ honest-disabled Cloud chips). */
function AppCard({
  app,
  pending,
  downloadPending,
  onRun,
  onView,
  onInspect,
  onOpen,
  onDownload,
  onDuplicate,
}: {
  app: AppSummary;
  pending: boolean;
  downloadPending: boolean;
  onRun: (handle: string) => void;
  onView: (handle: string) => void;
  onInspect: (handle: string) => void;
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
        <span className="card-grid__title">{app.name}</span>
        <span className="chip chip--tag">v{app.version}</span>
        {app.locked ? (
          <span
            className="chip chip--tag"
            data-testid={`app-card-locked-${app.handle}`}
            title="This App is locked — agentic in-CAS edits are refused (manage from the App page)"
          >
            🔒 Locked
          </span>
        ) : null}
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

      <div className="card-grid__meta">
        <button
          type="button"
          data-testid={`app-run-${app.handle}`}
          disabled={pending}
          onClick={() => onRun(app.handle)}
        >
          {pending ? "Running…" : "Run"}
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
                data-testid={`app-inspect-${app.handle}`}
                onClick={() => {
                  close();
                  onInspect(app.handle);
                }}
              >
                <Icon name="terminal" size={15} />
                <span>Inspect envelope</span>
              </button>
              <button
                type="button"
                role="menuitem"
                className="popover__item"
                data-testid={`app-download-${app.handle}`}
                disabled={downloadPending}
                title="Download a portable .kxapp bundle (envelope + content closure)"
                onClick={() => {
                  close();
                  onDownload(app.handle);
                }}
              >
                <Icon name="download" size={15} />
                <span>{downloadPending ? "Downloading…" : "Download bundle"}</span>
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
        <span className="chip chip--soon" title="Sharing across parties is a Cloud capability">
          Share · Cloud
        </span>
      </div>
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

/** A read-only slide-over showing one App's full `kortecx.app/v1` envelope
 *  (Monaco viewer; the BlueprintViewer precedent). */
function AppDetailDrawer({ handle, onClose }: { handle: string; onClose: () => void }) {
  const q = useApp(handle);

  useEffect(() => {
    function onKey(e: KeyboardEvent): void {
      if (e.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const envelope = q.data ? JSON.stringify(q.data.envelope, null, 2) : null;

  return (
    <>
      <button
        type="button"
        className="node-drawer__scrim"
        aria-label="Close app view"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer"
        data-testid="app-viewer"
        // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion; non-modal side-panel semantics via role+aria-label (the NodeDetailDrawer/BlueprintViewer precedent)
        role="dialog"
        aria-label={`App ${handle}`}
        initial={{ x: 24, opacity: 0 }}
        animate={{ x: 0, opacity: 1 }}
        transition={{ type: "spring", stiffness: 420, damping: 34 }}
      >
        <div className="node-drawer__head">
          <code className="mono node-drawer__id" title={handle}>
            {handle}
          </code>
          <button type="button" className="linkbtn" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>
        {q.isLoading ? <EmptyState title="Loading envelope…" /> : null}
        {q.error ? <ErrorNotice error={toUiError(q.error)} /> : null}
        {q.data === null ? <EmptyState title="Not found" /> : null}
        {envelope ? (
          <CodeViewer
            value={envelope}
            language="json"
            testId="app-envelope"
            ariaLabel={`App envelope ${handle}`}
            height={Math.min(520, Math.max(160, envelope.split("\n").length * 19 + 24))}
          />
        ) : null}
      </m.aside>
    </>
  );
}
