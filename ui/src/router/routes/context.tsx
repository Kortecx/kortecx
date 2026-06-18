import { createRoute } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

/**
 * Context — named, content-addressed bundles a caller attaches to a run (PR-7).
 * The section view (author + govern) loads lazily; it degrades to a not-wired
 * empty state on a gateway without the bundle store (don't-fake-gaps, GR15).
 */
const ContextSection = lazy(() =>
  import("../../components/sections/ContextSection").then((m) => ({ default: m.ContextSection })),
);

function ContextScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading…" />}>
      <ContextSection />
    </Suspense>
  );
}

export const contextRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/context",
  component: ContextScreen,
});
