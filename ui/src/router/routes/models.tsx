import { createRoute } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

// Lazy-loaded so the read-only Models catalog stays OFF the eager bundle (the
// ~600KiB budget) — first navigation fetches the chunk.
const ModelsSection = lazy(() =>
  import("../../components/sections/ModelsSection").then((m) => ({
    default: m.ModelsSection,
  })),
);

function ModelsScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading models…" />}>
      <ModelsSection />
    </Suspense>
  );
}

export const modelsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/models",
  component: ModelsScreen,
});
