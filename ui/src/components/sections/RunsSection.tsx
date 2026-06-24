import { Link } from "@tanstack/react-router";
import { WorkflowsTable } from "./WorkflowsTable";

/**
 * The Workflows home (POC-5c / D168): the runnable CATALOG — browse a blueprint
 * (workflow definition) and trigger a SINGLE run from its detail drawer. OSS runs
 * ONE App/blueprint at a time; multi-app chaining + model orchestration is a Cloud
 * capability (D129/GR19). Run HISTORY moved to Monitoring → Runs (the `RunsTable`
 * there), and the per-run live-DAG stays at `/workflows/$instanceId`. The frozen
 * section id stays `runs-section` (test-ids/telemetry never rename); old
 * `/workflows?view=runs` deep links land here on the catalog (history is in Monitoring).
 */
export function RunsSection() {
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
    </section>
  );
}
