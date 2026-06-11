import { createRoute } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const MonitoringSection = lazy(() =>
  import("../../components/sections/MonitoringSection").then((m) => ({
    default: m.MonitoringSection,
  })),
);

function MonitorScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading…" />}>
      <MonitoringSection />
    </Suspense>
  );
}

export const monitorRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/monitor",
  component: MonitorScreen,
});
