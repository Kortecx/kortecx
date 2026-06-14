import { createRoute, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const RunsSection = lazy(() =>
  import("../../components/sections/RunsSection").then((m) => ({ default: m.RunsSection })),
);

/** The Workflows page view-toggle (PR-A): the workflow DEFINITIONS table vs the
 *  RUN-history table — both live here (D141.1 one-home), URL-addressable. */
type WorkflowsView = "workflows" | "runs";
interface WorkflowsSearch {
  view?: WorkflowsView;
}

/**
 * The Workflows section home (PR-2 route merge, D141.1; PR-A tabs): a
 * `Workflows | Runs` toggle — workflow definitions + run history, both as
 * tables. The frozen section id stays `runs` (test-ids/telemetry never rename);
 * old `/runs` deep links redirect here.
 */
function WorkflowsScreen() {
  const { status } = useConnection();
  const { view } = useSearch({ from: "/workflows" });
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading workflows…" />}>
      <RunsSection view={view ?? "runs"} />
    </Suspense>
  );
}

export const workflowsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/workflows",
  validateSearch: (search: Record<string, unknown>): WorkflowsSearch => {
    const v = search.view;
    return v === "workflows" || v === "runs" ? { view: v } : {};
  },
  component: WorkflowsScreen,
});
