import { createRoute } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

/**
 * Branches — content-addressed `{path → ref}` manifests over operator-approved
 * host files (D155). The section view (govern + author) loads lazily; it degrades
 * to a not-wired empty state on a gateway without the branch store (GR15).
 */
const BranchesSection = lazy(() =>
  import("../../components/sections/BranchesSection").then((m) => ({
    default: m.BranchesSection,
  })),
);

function BranchesScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading…" />}>
      <BranchesSection />
    </Suspense>
  );
}

export const branchesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/branches",
  component: BranchesScreen,
});
