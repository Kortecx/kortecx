import { createRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const MonitoringSection = lazy(() =>
  import("../../components/sections/MonitoringSection").then((m) => ({
    default: m.MonitoringSection,
  })),
);

/** The Monitoring views (Batch C): the overview panels, the global live feed,
 *  and the execution-telemetry table. URL-addressable (the run-detail tab
 *  precedent); absent = "overview". */
const MONITOR_TABS = ["feed", "telemetry"] as const;
export type MonitorTab = (typeof MONITOR_TABS)[number];

interface MonitorSearch {
  /** The active monitoring view; absent = the overview panels. */
  tab?: MonitorTab;
}

function MonitorScreen() {
  const { status } = useConnection();
  const search = useSearch({ from: "/monitor" });
  const navigate = useNavigate({ from: "/monitor" });
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading…" />}>
      <MonitoringSection
        tab={search.tab}
        onTab={(tab) => void navigate({ search: { tab }, replace: true })}
      />
    </Suspense>
  );
}

export const monitorRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/monitor",
  component: MonitorScreen,
  validateSearch: (search: Record<string, unknown>): MonitorSearch => {
    const out: MonitorSearch = {};
    if (
      typeof search.tab === "string" &&
      (MONITOR_TABS as readonly string[]).includes(search.tab)
    ) {
      out.tab = search.tab as MonitorTab;
    }
    return out;
  },
});
