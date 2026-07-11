import { createRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import type { AppsTab } from "../../components/sections/AppsSection";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const AppsSection = lazy(() =>
  import("../../components/sections/AppsSection").then((m) => ({ default: m.AppsSection })),
);

/** The Apps view; absent = the App catalog, `approvals` = the cross-App HITL inbox. */
interface AppsSearch {
  tab?: "approvals";
}

function AppsScreen() {
  const { status } = useConnection();
  const search = useSearch({ from: "/apps" });
  const navigate = useNavigate({ from: "/apps" });
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading apps…" />}>
      <AppsSection
        tab={search.tab ?? "catalog"}
        onTab={(tab: AppsTab) =>
          void navigate({ search: tab === "approvals" ? { tab } : {}, replace: true })
        }
      />
    </Suspense>
  );
}

export const appsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/apps",
  component: AppsScreen,
  validateSearch: (search: Record<string, unknown>): AppsSearch =>
    search.tab === "approvals" ? { tab: "approvals" } : {},
});
