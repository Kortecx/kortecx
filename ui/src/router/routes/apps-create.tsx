import { createRoute, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

// The create journey (form → live scaffold → terminal result) is route-lazy — this
// eager route module carries only the registration (the blueprints-new precedent).
const CreateAppScreen = lazy(() =>
  import("../../components/sections/CreateAppScreen").then((m) => ({
    default: m.CreateAppScreen,
  })),
);

/** `?kind=hosted` preselects the hosted lane (the Apps page's own section param
 *  convention); absent ⇒ scheduled. The form's own toggle stays the authority. */
interface CreateAppSearch {
  kind?: "hosted";
}

function CreateScreen() {
  const { status } = useConnection();
  const { kind } = useSearch({ from: "/apps/create" });
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading…" />}>
      <CreateAppScreen initialKind={kind === "hosted" ? "hosted" : "scheduled"} />
    </Suspense>
  );
}

export const appsCreateRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/apps/create",
  validateSearch: (search: Record<string, unknown>): CreateAppSearch => {
    const out: CreateAppSearch = {};
    if (search.kind === "hosted") {
      out.kind = "hosted";
    }
    return out;
  },
  component: CreateScreen,
});
