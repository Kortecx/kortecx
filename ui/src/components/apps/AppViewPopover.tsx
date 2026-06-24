import { m } from "framer-motion";
import { useEffect } from "react";
import { toUiError } from "../../kx/errors";
import { useApp } from "../../kx/use-apps";
import { useBranches } from "../../kx/use-branches";
import { shortHex } from "../../lib/format";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";

/**
 * POC-5c: the Apps "View" popup — a READ-ONLY, at-a-glance summary of one App: its
 * `kortecx.app/v1` envelope summary (name, version, steps, tags, lock, server-derived
 * appRef) PLUS a project-branch / lineage snapshot. Distinct from "Inspect" (the raw
 * envelope JSON via Monaco) and from "Open" (the full FileTree+Monaco IDE — the
 * in-window editor + lineage-graph editing is POC-5d). Composes existing hooks only
 * (no new RPC). One-App-one-branch: the project branch shares the App's handle, so the
 * lineage snapshot is found by handle (an honest empty state when not yet scaffolded).
 */
export function AppViewPopover({ handle, onClose }: { handle: string; onClose: () => void }) {
  const app = useApp(handle);
  const { branches, notWired: branchesNotWired } = useBranches();
  const branch = branches.find((b) => b.handle === handle);

  useEffect(() => {
    function onKey(e: KeyboardEvent): void {
      if (e.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const summary = app.data?.summary;

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
        data-testid="app-view"
        // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion; non-modal side-panel semantics via role+aria-label (the AppDetailDrawer precedent)
        role="dialog"
        aria-label={`App ${handle} details`}
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

        {app.isLoading ? <EmptyState title="Loading…" /> : null}
        {app.error ? <ErrorNotice error={toUiError(app.error)} /> : null}
        {app.data === null ? <EmptyState title="Not found" /> : null}

        {summary ? (
          <>
            <dl className="facts" data-testid="app-view-summary">
              <dt>Name</dt>
              <dd>{summary.name}</dd>
              <dt>Version</dt>
              <dd>v{summary.version}</dd>
              <dt>App ref</dt>
              <dd className="mono" title={summary.appRef}>
                {shortHex(summary.appRef)}
              </dd>
              <dt>Steps</dt>
              <dd>{summary.stepCount}</dd>
              <dt>Lock</dt>
              <dd>{summary.locked ? "🔒 locked (agent-write refused)" : "unlocked"}</dd>
              {summary.description ? (
                <>
                  <dt>Description</dt>
                  <dd>{summary.description}</dd>
                </>
              ) : null}
              {summary.tags.length > 0 ? (
                <>
                  <dt>Tags</dt>
                  <dd className="card-grid__tags">
                    {summary.tags.map((t) => (
                      <span key={t} className="chip chip--tag">
                        {t}
                      </span>
                    ))}
                  </dd>
                </>
              ) : null}
            </dl>

            <h3 className="node-drawer__section">Project branch (lineage)</h3>
            {branch ? (
              <dl className="facts" data-testid="app-view-branch">
                <dt>Files</dt>
                <dd>{branch.itemCount}</dd>
                <dt>Branch ref</dt>
                <dd className="mono" title={branch.branchRef}>
                  {shortHex(branch.branchRef)}
                </dd>
                <dt>Parent</dt>
                <dd className="mono">
                  {branch.parentHandle === "" ? "(root)" : branch.parentHandle}
                </dd>
              </dl>
            ) : (
              <p className="muted" data-testid="app-view-no-branch">
                {branchesNotWired
                  ? "Branch store not wired on this gateway."
                  : "No project branch yet — scaffold this App (New App) to author its files."}
              </p>
            )}
          </>
        ) : null}
      </m.aside>
    </>
  );
}
