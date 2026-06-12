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
 * The Workflows section home (PR-2 route merge, D141.1): the run list. The
 * frozen section id stays `runs` (test-ids/telemetry never rename); old
 * `/runs` deep links redirect here.
 */
function WorkflowsScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading runs…" />}>
      <RunsSection />
    </Suspense>
  );
}

export const workflowsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/workflows",
  component: WorkflowsScreen,
});
