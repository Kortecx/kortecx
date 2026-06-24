import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { useApps } from "../../kx/use-apps";
import { AppRunDrawer } from "../apps/AppRunDrawer";
import { WorkflowsTable } from "./WorkflowsTable";

/**
 * The Workflows home (POC-5c / D168): the runnable CATALOG — browse a blueprint
 * (workflow definition) and trigger a SINGLE run from its detail drawer, plus the
 * (POC-5d) Apps trigger list: each saved App is runnable ONE at a time from here
 * (its `input_schema` inputs collected in the run drawer). OSS runs ONE App/blueprint
 * at a time; multi-app chaining + model orchestration is a Cloud capability
 * (D129/GR19). Run HISTORY moved to Monitoring → Runs; the per-run live-DAG stays at
 * `/workflows/$instanceId`. The frozen section id stays `runs-section`.
 */
export function RunsSection() {
  const { apps, notWired } = useApps();
  const [runHandle, setRunHandle] = useState<string | null>(null);

  return (
    <section className="screen" data-testid="runs-section">
      <div className="section-head">
        <div>
          <h1>Workflows</h1>
          <p className="muted">
            Browse a blueprint and trigger a run. Run history & live telemetry live in{" "}
            <Link to="/monitor" search={{ tab: "runs" }} data-testid="workflows-to-monitor">
              Monitoring → Runs
            </Link>
            .
          </p>
        </div>
        <Link to="/recipes" className="btn-ghost" data-testid="workflows-browse-blueprints">
          Browse blueprints
        </Link>
      </div>

      <WorkflowsTable />

      {!notWired && apps.length > 0 ? (
        <div className="runs-apps" data-testid="runs-apps">
          <h2>Apps</h2>
          <p className="muted">Trigger a saved App as a single run, or open it in the IDE.</p>
          <ul className="runs-apps__list">
            {apps.map((a) => (
              <li key={a.handle} className="runs-apps__row" data-testid={`runs-app-${a.handle}`}>
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

      {runHandle ? <AppRunDrawer handle={runHandle} onClose={() => setRunHandle(null)} /> : null}
    </section>
  );
}
