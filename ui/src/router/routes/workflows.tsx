import { createRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import type { WorkflowsTab } from "../../components/sections/RunsSection";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const RunsSection = lazy(() =>
  import("../../components/sections/RunsSection").then((m) => ({ default: m.RunsSection })),
);

/**
 * The Workflows section home (POC-5c / D168): absent = the runnable blueprint/App
 * CATALOG, `runs` = your own run history, `trails` = the self-correction trails.
 * A stale `?view=` deep link (D141.1 era) is dropped, landing on the catalog. The
 * frozen section id stays `runs`.
 */
interface WorkflowsSearch {
  tab?: "runs" | "trails";
}

function WorkflowsScreen() {
  const { status } = useConnection();
  const search = useSearch({ from: "/workflows" });
  const navigate = useNavigate({ from: "/workflows" });
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading workflows…" />}>
      <RunsSection
        tab={search.tab ?? "catalog"}
        onTab={(tab: WorkflowsTab) =>
          void navigate({ search: tab === "catalog" ? {} : { tab }, replace: true })
        }
      />
    </Suspense>
  );
}

export const workflowsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/workflows",
  validateSearch: (search: Record<string, unknown>): WorkflowsSearch =>
    search.tab === "runs" || search.tab === "trails" ? { tab: search.tab } : {},
  component: WorkflowsScreen,
});
