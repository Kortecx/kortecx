import { createRoute, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

// The create journey (header form + the embedded builder canvas) is route-lazy —
// this eager route module carries only the registration (the apps-create precedent).
const CreateWorkflowScreen = lazy(() =>
  import("../../components/sections/CreateWorkflowScreen").then((m) => ({
    default: m.CreateWorkflowScreen,
  })),
);

/** `?handle=` seeds the form + canvas from a saved Workflow (`GetWorkflow`) — the
 *  edit / finish-draft entry. Absent ⇒ a fresh single-agent starting graph. */
interface CreateWorkflowSearch {
  handle?: string;
}

function CreateScreen() {
  const { status } = useConnection();
  const { handle } = useSearch({ from: "/workflows/create" });
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading…" />}>
      <CreateWorkflowScreen seedHandle={handle ?? null} />
    </Suspense>
  );
}

export const workflowsCreateRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/workflows/create",
  validateSearch: (search: Record<string, unknown>): CreateWorkflowSearch => {
    const out: CreateWorkflowSearch = {};
    if (typeof search.handle === "string" && search.handle !== "") {
      out.handle = search.handle;
    }
    return out;
  },
  component: CreateScreen,
});
