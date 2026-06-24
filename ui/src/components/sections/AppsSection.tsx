import type { AppSummary } from "@kortecx/sdk/web";
import { useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useEffect, useState } from "react";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useApp, useApps, useRunApp } from "../../kx/use-apps";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { CodeViewer } from "../editor/CodeViewer";

/**
 * The Apps catalog (POC-4) — a READ-ONLY view over the caller's durable
 * `kortecx.app/v1` envelopes (`ListApps`). Each App is a portable blueprint
 * wrapped with by-reference references + a 4-axis steering config; Run compiles
 * its blueprint and submits it (the server re-resolves every warrant from the
 * caller's grants, SN-8). Inspect shows the full envelope (Monaco, read-only).
 *
 * Authoring is the SDK/CLI surface (`kx app` / `app()`); the agentic "New App"
 * scaffold + in-CAS file editing land in POC-5a — so there is no New-App button
 * here yet (GR15 don't-fake-gaps). Share + Schedule are Cloud (D129) — honest-
 * disabled chips, never fake controls. Nothing currently exposed disappears:
 * Apps is ADDITIVE alongside Workflows/Blueprints (the consolidation rides the
 * deferred redesign).
 */
export function AppsSection() {
  const navigate = useNavigate();
  const { apps, notWired, isLoading, isError, error, refetch } = useApps();
  const runApp = useRunApp();
  const [viewing, setViewing] = useState<string | null>(null);

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

  return (
    <section className="screen" data-testid="apps-section">
      <div className="section-head">
        <div>
          <h1>Apps</h1>
          <p className="muted">
            Durable, reusable Apps — a portable blueprint plus its references, steering, and replay
            intent. Author with <code className="mono">kx app</code> or the{" "}
            <code className="mono">app()</code> SDK; run one here. Building + editing an App in-CAS
            arrives in a later release.
          </p>
        </div>
      </div>

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
              onInspect={setViewing}
            />
          ))}
        </m.div>
      ) : null}

      {runError ? <ErrorNotice error={runError} onRetry={() => runApp.reset()} /> : null}

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
  onInspect,
}: {
  app: AppSummary;
  pending: boolean;
  onRun: (handle: string) => void;
  onInspect: (handle: string) => void;
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
          data-testid={`app-inspect-${app.handle}`}
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
