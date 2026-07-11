import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { useApps } from "../../kx/use-apps";
import { AppRunDrawer } from "../apps/AppRunDrawer";
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
 * The Workflows home (POC-5c / D168): the runnable CATALOG — browse a blueprint
 * (workflow definition) and trigger a SINGLE run from its detail drawer, plus the
 * (POC-5d) Apps trigger list: each saved App is runnable ONE at a time from here
 * (its `input_schema` inputs collected in the run drawer). Also your own run HISTORY
 * and the self-correction TRAILS (react / replan / rerank / capture) for those runs;
 * the per-run live-DAG stays at `/workflows/$instanceId`. A run triggers one
 * App/blueprint at a time. The frozen section id stays `runs-section`. Tab state rides
 * the route's validated search.
 */
export function RunsSection({
  tab = "catalog",
  onTab,
}: {
  tab?: WorkflowsTab;
  onTab?: (tab: WorkflowsTab) => void;
} = {}) {
  const { apps, notWired } = useApps();
  const [runHandle, setRunHandle] = useState<string | null>(null);

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

          {!notWired && apps.length > 0 ? (
            <div className="runs-apps" data-testid="runs-apps">
              <h2>Apps</h2>
              <p className="muted">Trigger a saved App as a single run, or open it in the IDE.</p>
              <ul className="runs-apps__list">
                {apps.map((a) => (
                  <li
                    key={a.handle}
                    className="runs-apps__row"
                    data-testid={`runs-app-${a.handle}`}
                  >
                    <div className="runs-apps__id">
                      <span className="runs-apps__name">{a.name}</span>
                      <code className="mono muted">{a.handle}</code>
                    </div>
                    <div className="runs-apps__actions">
                      {a.locked ? (
                        <span className="chip chip--tag" title="Edits are refused">
                          🔒
                        </span>
                      ) : null}
                      <Link
                        to="/apps/$handle"
                        params={{ handle: a.handle }}
                        className="btn-ghost"
                        data-testid={`runs-app-open-${a.handle}`}
                      >
                        Open
                      </Link>
                      <button
                        type="button"
                        className="btn-primary"
                        data-testid={`runs-app-run-${a.handle}`}
                        onClick={() => setRunHandle(a.handle)}
                      >
                        Run
                      </button>
                    </div>
                  </li>
                ))}
              </ul>
            </div>
          ) : null}

          {runHandle ? (
            <AppRunDrawer handle={runHandle} onClose={() => setRunHandle(null)} />
          ) : null}
        </>
      )}
    </section>
  );
}
