import { createRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

/**
 * Context — the data & storage umbrella (POC-5c / D168): reusable bundles (PR-7) plus
 * the Datasets / Data Lab tab (T3.7). The section view loads lazily; each tab degrades
 * to a not-wired empty state on a gateway without that store (don't-fake-gaps, GR15).
 */
const ContextSection = lazy(() =>
  import("../../components/sections/ContextSection").then((m) => ({ default: m.ContextSection })),
);

/** The Context tabs: bundles (default, absent) and the RAG datasets / Data Lab. */
export type ContextTab = "bundles" | "datasets";
interface ContextSearch {
  /** The active tab; absent = the Bundles tab. */
  tab?: "datasets";
}

function ContextScreen() {
  const { status } = useConnection();
  const search = useSearch({ from: "/context" });
  const navigate = useNavigate({ from: "/context" });
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading…" />}>
      <ContextSection
        tab={search.tab ?? "bundles"}
        onTab={(tab) => void navigate({ search: tab === "datasets" ? { tab } : {}, replace: true })}
      />
    </Suspense>
  );
}

export const contextRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/context",
  component: ContextScreen,
  validateSearch: (search: Record<string, unknown>): ContextSearch =>
    search.tab === "datasets" ? { tab: "datasets" } : {},
});
