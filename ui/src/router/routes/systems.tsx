import { createRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const SystemsSection = lazy(() =>
  import("../../components/sections/SystemsSection").then((m) => ({ default: m.SystemsSection })),
);

/** The selected App whose capability manifest the Security section resolves; absent =
 *  the first App. */
interface SecuritySearch {
  handle?: string;
}

function SystemsScreen() {
  const { status } = useConnection();
  const search = useSearch({ from: "/systems" });
  const navigate = useNavigate({ from: "/systems" });
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading…" />}>
      <SystemsSection
        handle={search.handle}
        onHandle={(handle) => void navigate({ search: { handle }, replace: true })}
      />
    </Suspense>
  );
}

export const systemsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/systems",
  component: SystemsScreen,
  validateSearch: (search: Record<string, unknown>): SecuritySearch =>
    typeof search.handle === "string" ? { handle: search.handle } : {},
});
