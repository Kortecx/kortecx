import { Link } from "@tanstack/react-router";
import { RunsTable } from "./RunsTable";
import { WorkflowTrails } from "./WorkflowTrails";
import { WorkflowsTable } from "./WorkflowsTable";

/** The Workflows section views: the runnable blueprint/App CATALOG (default), your own
 *  run HISTORY, and the self-correction TRAILS for those runs. */
export type WorkflowsTab = "catalog" | "runs" | "trails";

const WORKFLOWS_TABS: ReadonlyArray<{ id: WorkflowsTab; label: string }> = [
  { id: "catalog", label: "Catalog" },
  { id: "runs", label: "Runs" },
  { id: "trails", label: "Trails" },
];

/**
 * The Workflows home (POC-5c / D168): the runnable blueprint CATALOG — browse a
 * blueprint (workflow definition) and trigger a SINGLE run from its detail drawer —
 * plus your own run HISTORY and the self-correction TRAILS (react / replan / rerank /
 * capture) for those runs; the per-run live-DAG stays at `/workflows/$instanceId`.
 * WAVE-3: saved Apps are no longer duplicated here — they have one home in the Apps
 * section (this catalog links there), where an App runs from its typed input drawer.
 * The frozen section id stays `runs-section`. Tab state rides the route's validated search.
 */
export function RunsSection({
  tab = "catalog",
  onTab,
}: {
  tab?: WorkflowsTab;
  onTab?: (tab: WorkflowsTab) => void;
} = {}) {
  return (
    <section className="screen" data-testid="runs-section">
      <div className="section-head">
        <div>
          <h1>Workflows</h1>
          <p className="muted">
            Browse a blueprint and trigger a run, review your run history, or trace a run's
            self-correction.
          </p>
        </div>
        <Link to="/recipes" className="btn-ghost" data-testid="workflows-browse-blueprints">
          Browse blueprints
        </Link>
      </div>

      <fieldset className="view-toggle" aria-label="Workflows view" data-testid="workflows-tabs">
        {WORKFLOWS_TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            data-testid={`workflows-tab-${t.id}`}
            aria-pressed={tab === t.id}
            onClick={() => onTab?.(t.id)}
          >
            {t.label}
          </button>
        ))}
      </fieldset>

      {tab === "runs" ? (
        <div data-testid="workflows-runs">
          <RunsTable />
        </div>
      ) : tab === "trails" ? (
        <WorkflowTrails />
      ) : (
        <>
          <WorkflowsTable />
          <p className="muted" data-testid="workflows-apps-hint">
            Looking for a saved App? Run, create, and manage Apps in the{" "}
            <Link to="/apps" data-testid="workflows-apps-link">
              Apps
            </Link>{" "}
            section — each App runs from its typed input drawer.
          </p>
        </>
      )}
    </section>
  );
}
