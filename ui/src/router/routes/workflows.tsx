import { createRoute } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const RunsSection = lazy(() =>
  import("../../components/sections/RunsSection").then((m) => ({ default: m.RunsSection })),
);

/**
 * The Workflows section home (POC-5c / D168): the runnable blueprint CATALOG. Run
 * history moved to Monitoring → Runs, so the old `?view=runs` toggle is gone — the
 * search param is accepted-but-ignored so stale `/workflows?view=runs` deep links
 * land here (the catalog) instead of 404ing. The frozen section id stays `runs`.
 */
function WorkflowsScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading workflows…" />}>
      <RunsSection />
    </Suspense>
  );
}

export const workflowsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/workflows",
  // Tolerant of stale `?view=` deep links (D141.1 era): accept any search and
  // drop it — the Workflows section is now the catalog only (run history → Monitoring).
  validateSearch: (): Record<string, never> => ({}),
  component: WorkflowsScreen,
});
