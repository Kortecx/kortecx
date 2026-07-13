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

/** The Apps view; `tab` absent = the App catalog, `approvals` = the cross-App HITL
 *  inbox. `view` = the catalog layout (`list` rows vs the default box/card grid). */
interface AppsSearch {
  tab?: "approvals";
  view?: "list";
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
          void navigate({
            search: (prev) => ({ ...prev, tab: tab === "approvals" ? "approvals" : undefined }),
            replace: true,
          })
        }
        view={search.view ?? "box"}
        onView={(view) =>
          void navigate({
            search: (prev) => ({ ...prev, view: view === "list" ? "list" : undefined }),
            replace: true,
          })
        }
      />
    </Suspense>
  );
}

export const appsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/apps",
  component: AppsScreen,
  validateSearch: (search: Record<string, unknown>): AppsSearch => {
    const out: AppsSearch = {};
    if (search.tab === "approvals") {
      out.tab = "approvals";
    }
    if (search.view === "list") {
      out.view = "list";
    }
    return out;
  },
});
