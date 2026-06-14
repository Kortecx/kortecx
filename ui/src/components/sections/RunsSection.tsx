import { useNavigate } from "@tanstack/react-router";
import { RunsTable } from "./RunsTable";
import { WorkflowsTable } from "./WorkflowsTable";

type WorkflowsView = "workflows" | "runs";

const TABS: ReadonlyArray<{ id: WorkflowsView; label: string }> = [
  { id: "workflows", label: "Workflows" },
  { id: "runs", label: "Runs" },
];

/**
 * The Workflows home (PR-A): a `Workflows | Runs` page (D141.1 one-home) — the
 * workflow DEFINITIONS table and the RUN-history table both live here, behind a
 * URL-addressable view-toggle (`?view=`, the D142.2 tab pattern). The frozen
 * section id stays `runs-section` (test-ids/telemetry never rename).
 */
export function RunsSection({ view }: { view: WorkflowsView }) {
  const navigate = useNavigate();

  return (
    <section className="screen" data-testid="runs-section">
      <div className="screen__head">
        <h1>Workflows</h1>
      </div>

      <fieldset className="view-toggle" data-testid="workflows-tabs" aria-label="Workflows view">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            data-testid={`workflows-tab-${t.id}`}
            aria-pressed={view === t.id}
            onClick={() =>
              void navigate({ to: "/workflows", search: t.id === "runs" ? {} : { view: t.id } })
            }
          >
            {t.label}
          </button>
        ))}
      </fieldset>

      {view === "workflows" ? <WorkflowsTable /> : <RunsTable />}
    </section>
  );
}
