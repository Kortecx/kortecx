import { createRoute } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const DatasetsSection = lazy(() =>
  import("../../components/sections/DatasetsSection").then((m) => ({ default: m.DatasetsSection })),
);

function DatasetsScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading…" />}>
      <DatasetsSection />
    </Suspense>
  );
}

export const datasetsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/datasets",
  component: DatasetsScreen,
});
