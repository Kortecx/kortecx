import { createRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const ToolsSection = lazy(() =>
  import("../../components/sections/ToolsSection").then((m) => ({ default: m.ToolsSection })),
);

/** The Integrations tabs: tools (default, absent), connections, skills, triggers, secrets. */
export type ToolsTab = "tools" | "connections" | "skills" | "triggers" | "secrets";
/** The non-default tabs carried in the route search (`tools` is the absent default). */
type ToolsTabSearch = Exclude<ToolsTab, "tools">;
const TAB_SEARCH: readonly ToolsTabSearch[] = ["connections", "skills", "triggers", "secrets"];
interface ToolsSearch {
  /** The active tab; absent = the Tools tab. */
  tab?: ToolsTabSearch;
}

function ToolsScreen() {
  const { status } = useConnection();
  const search = useSearch({ from: "/tools" });
  const navigate = useNavigate({ from: "/tools" });
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading…" />}>
      <ToolsSection
        tab={search.tab ?? "tools"}
        onTab={(tab) => void navigate({ search: tab === "tools" ? {} : { tab }, replace: true })}
      />
    </Suspense>
  );
}

export const toolsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/tools",
  component: ToolsScreen,
  validateSearch: (search: Record<string, unknown>): ToolsSearch =>
    TAB_SEARCH.includes(search.tab as ToolsTabSearch) ? { tab: search.tab as ToolsTabSearch } : {},
});
