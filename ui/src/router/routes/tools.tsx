import { createRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const ToolsSection = lazy(() =>
  import("../../components/sections/ToolsSection").then((m) => ({ default: m.ToolsSection })),
);

/** The MCP tabs: tools (default, absent), scripts, connectors, skills, triggers, secrets. */
export type ToolsTab = "tools" | "scripts" | "connectors" | "skills" | "triggers" | "secrets";
/** The non-default tabs carried in the route search (`tools` is the absent default). */
type ToolsTabSearch = Exclude<ToolsTab, "tools">;
const TAB_SEARCH: readonly ToolsTabSearch[] = [
  "scripts",
  "connectors",
  "skills",
  "triggers",
  "secrets",
];
/** Links to the two tabs that merged into Connectors still resolve, rather than silently
 *  dropping to the default tab — a bookmark is not a reason to lose someone's place. */
const MERGED_INTO_CONNECTORS: readonly string[] = ["integrations", "connections"];
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
  validateSearch: (search: Record<string, unknown>): ToolsSearch => {
    const tab = search.tab as ToolsTabSearch;
    if (TAB_SEARCH.includes(tab)) {
      return { tab };
    }
    return MERGED_INTO_CONNECTORS.includes(search.tab as string) ? { tab: "connectors" } : {};
  },
});
