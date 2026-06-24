import { createRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const SystemsSection = lazy(() =>
  import("../../components/sections/SystemsSection").then((m) => ({ default: m.SystemsSection })),
);

/** The Security tabs (POC-5c / D168): teams & grants (default, absent) and the
 *  per-App Policies lock surface. */
export type SecurityTab = "teams" | "policies";
interface SecuritySearch {
  /** The active tab; absent = the Teams & grants viewers. */
  tab?: "policies";
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
        tab={search.tab ?? "teams"}
        onTab={(tab) => void navigate({ search: tab === "policies" ? { tab } : {}, replace: true })}
      />
    </Suspense>
  );
}

export const systemsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/systems",
  component: SystemsScreen,
  validateSearch: (search: Record<string, unknown>): SecuritySearch =>
    search.tab === "policies" ? { tab: "policies" } : {},
});
