import type { AppSummary } from "@kortecx/sdk/web";
import { useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useEffect, useState } from "react";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useApp, useApps, useRunApp } from "../../kx/use-apps";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { AppViewPopover } from "../apps/AppViewPopover";
import { CodeViewer } from "../editor/CodeViewer";
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
export function AppsSection() {
  const navigate = useNavigate();
  const { apps, notWired, isLoading, isError, error, refetch } = useApps();
  const runApp = useRunApp();
  const [viewing, setViewing] = useState<string | null>(null);
  const [summaryFor, setSummaryFor] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

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
              onRun={run}
              onView={setSummaryFor}
              onInspect={setViewing}
              onOpen={(handle) => void navigate({ to: "/apps/$handle", params: { handle } })}
            />
          ))}
        </m.div>
      ) : null}

      {runError ? (
        <ErrorNotice error={runError} onRetry={() => runApp.reset()} action={runAction} />
      ) : null}

      {summaryFor ? (
        <AppViewPopover handle={summaryFor} onClose={() => setSummaryFor(null)} />
      ) : null}
      {viewing ? <AppDetailDrawer handle={viewing} onClose={() => setViewing(null)} /> : null}
    </section>
  );
}

/** One App in the catalog — name, description, version/step/tag chips, the raw
 *  handle, and Run / Inspect actions (+ honest-disabled Cloud chips). */
function AppCard({
  app,
  pending,
  onRun,
  onView,
  onInspect,
  onOpen,
}: {
  app: AppSummary;
  pending: boolean;
  onRun: (handle: string) => void;
  onView: (handle: string) => void;
  onInspect: (handle: string) => void;
  onOpen: (handle: string) => void;
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
            title="This App is locked — agentic in-CAS edits are refused (manage in Policies)"
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
        <button
          type="button"
          className="btn-ghost"
          data-testid={`app-view-${app.handle}`}
          title="View details — the envelope summary & project-branch lineage (read-only)"
          onClick={() => onView(app.handle)}
        >
          View
        </button>
        <button
          type="button"
          className="btn-ghost"
          data-testid={`app-open-${app.handle}`}
          title="Open the App — browse & edit its project files"
          onClick={() => onOpen(app.handle)}
        >
          Open
        </button>
        <button
          type="button"
          className="btn-ghost"
          data-testid={`app-inspect-${app.handle}`}
          title="Inspect the raw kortecx.app/v1 envelope (JSON)"
          onClick={() => onInspect(app.handle)}
        >
          Inspect
        </button>
        <span className="chip chip--soon" title="Sharing across parties is a Cloud capability">
          Share · Cloud
        </span>
      </div>
    </m.article>
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
