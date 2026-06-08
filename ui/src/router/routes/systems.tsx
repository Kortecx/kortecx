import { createRoute } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const SystemsSection = lazy(() =>
  import("../../components/sections/SystemsSection").then((m) => ({ default: m.SystemsSection })),
);

function SystemsScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading…" />}>
      <SystemsSection />
    </Suspense>
  );
}

export const systemsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/systems",
  component: SystemsScreen,
});
