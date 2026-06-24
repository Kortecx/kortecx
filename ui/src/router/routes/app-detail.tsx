import { createRoute, useParams } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const ROUTE_ID = "/apps/$handle";

const AppDetailSection = lazy(() =>
  import("../../components/sections/AppDetailSection").then((m) => ({
    default: m.AppDetailSection,
  })),
);

function AppDetailScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return <AppDetailContent />;
}

function AppDetailContent() {
  const { handle } = useParams({ from: ROUTE_ID });
  return (
    <Suspense fallback={<EmptyState title="Loading app…" />}>
      <AppDetailSection handle={handle} />
    </Suspense>
  );
}

export const appDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: ROUTE_ID,
  component: AppDetailScreen,
});
