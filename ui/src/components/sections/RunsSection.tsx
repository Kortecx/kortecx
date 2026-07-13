import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { useConnection } from "../../kx/connection-context";
import { queryKeys } from "../../kx/query-keys";
import { Icon } from "../shell/Icon";
import { RunsTable } from "./RunsTable";
import { WorkflowsCatalog } from "./WorkflowsCatalog";
import { WorkflowsTemplatesPanel } from "./WorkflowsTemplatesPanel";

/** The Workflows section views: the runnable workflow CATALOG (default), your own
 *  one-time RUN history, and reusable TEMPLATES (placeholder — the next increment). */
export type WorkflowsTab = "catalog" | "runs" | "templates";

const WORKFLOWS_TABS: ReadonlyArray<{ id: WorkflowsTab; label: string }> = [
  { id: "catalog", label: "Catalog" },
  { id: "runs", label: "Runs" },
  { id: "templates", label: "Templates" },
];

/**
 * The Workflows home (POC-5c / D168): the runnable workflow CATALOG — each workflow
 * as a high-level card (name · description · Run/Schedule/Share) — plus your one-time
 * RUN history and a placeholder for the reusable TEMPLATES to come. A single
 * top-right cluster holds Refresh + New workflow (the visual builder). The per-run
 * live-DAG stays at `/workflows/$instanceId`. The frozen section id stays
 * `runs-section`; tab state rides the route's validated search.
 */
export function RunsSection({
  tab = "catalog",
  onTab,
}: {
  tab?: WorkflowsTab;
  onTab?: (tab: WorkflowsTab) => void;
} = {}) {
  const { endpoint } = useConnection();
  const qc = useQueryClient();
  const [refreshing, setRefreshing] = useState(false);

  // The single Refresh action: re-pull the catalog and the run history. Only the
  // active tab's query actually refetches (react-query invalidation); the others go
  // stale and refresh on next mount.
  async function refresh(): Promise<void> {
    setRefreshing(true);
    try {
      await Promise.all([
        qc.invalidateQueries({ queryKey: queryKeys.recipes(endpoint) }),
        qc.invalidateQueries({ queryKey: queryKeys.recipeSummaries(endpoint) }),
        qc.invalidateQueries({ queryKey: ["kx", endpoint, "runs"] }),
      ]);
    } finally {
      setRefreshing(false);
    }
  }

  return (
    <section className="screen" data-testid="runs-section">
      <div className="section-head">
        <div>
          <h1>Workflows</h1>
        </div>
        <div className="section-head__actions">
          <button
            type="button"
            className="btn-ghost"
            data-testid="workflows-refresh"
            disabled={refreshing}
            title="Re-pull the workflow catalog and run history"
            onClick={() => void refresh()}
          >
            <Icon name="refresh" size={15} />
            <span>{refreshing ? "Refreshing…" : "Refresh"}</span>
          </button>
          <Link to="/blueprints/new" className="btn-primary" data-testid="workflows-new">
            <Icon name="plus" size={15} />
            <span>New workflow</span>
          </Link>
        </div>
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
      ) : tab === "templates" ? (
        <WorkflowsTemplatesPanel />
      ) : (
        <WorkflowsCatalog />
      )}
    </section>
  );
}
