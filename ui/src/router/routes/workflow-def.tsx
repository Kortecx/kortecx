import { createRoute, useParams } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

// The def detail is route-lazy — this eager module carries only the registration.
const WorkflowDefSection = lazy(() =>
  import("../../components/sections/WorkflowDefSection").then((m) => ({
    default: m.WorkflowDefSection,
  })),
);

/**
 * The stored-Workflow DEFINITION page. `/workflows/$instanceId` already MEANS
 * run-instance (instance ids are hex, so the static `def` segment can never
 * collide with one) — the definition lives under its own `/workflows/def/$handle`
 * home instead of overloading the run route.
 */
const ROUTE_ID = "/workflows/def/$handle";

function WorkflowDefScreen() {
  const { status } = useConnection();
  const { handle } = useParams({ from: ROUTE_ID });
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading workflow…" />}>
      <WorkflowDefSection handle={handle} />
    </Suspense>
  );
}

export const workflowDefRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: ROUTE_ID,
  component: WorkflowDefScreen,
});
