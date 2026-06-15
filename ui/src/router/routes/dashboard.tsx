import { createRoute } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

// Lazy-loaded so the landing stays OFF the eager bundle (the ~600KiB budget).
const DashboardSection = lazy(() =>
  import("../../components/sections/DashboardSection").then((m) => ({
    default: m.DashboardSection,
  })),
);

function DashboardScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading dashboard…" />}>
      <DashboardSection />
    </Suspense>
  );
}

export const dashboardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/dashboard",
  component: DashboardScreen,
});
