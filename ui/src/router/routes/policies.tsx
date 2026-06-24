import { createRoute } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const PoliciesSection = lazy(() =>
  import("../../components/sections/PoliciesSection").then((m) => ({
    default: m.PoliciesSection,
  })),
);

function PoliciesScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading policies…" />}>
      <PoliciesSection />
    </Suspense>
  );
}

export const policiesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/policies",
  component: PoliciesScreen,
});
