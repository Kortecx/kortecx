import { createRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { Suspense, lazy, useEffect } from "react";
import { useApprovalsDrawer } from "../../app/approvals-context";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import type { AppsSectionKind } from "../../components/sections/AppsSection";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const AppsSection = lazy(() =>
  import("../../components/sections/AppsSection").then((m) => ({ default: m.AppsSection })),
);

/** The Apps view. `section` absent = Scheduled (functional) apps, `hosted` = Hosted
 *  (experience) apps. `view` = the catalog layout (`table` vs the default box/card grid).
 *  `tab=approvals` is a back-compat deep-link migrated to the navbar approvals bell. */
interface AppsSearch {
  section?: "hosted";
  view?: "table";
  tab?: "approvals";
}

function AppsScreen() {
  const { status } = useConnection();
  const search = useSearch({ from: "/apps" });
  const navigate = useNavigate({ from: "/apps" });
  const approvals = useApprovalsDrawer();

  // Migrate the retired `/apps?tab=approvals` deep-link: open the navbar approvals
  // drawer once and strip the param (approvals now live on the bell, not a tab).
  const openApprovals = approvals.show;
  useEffect(() => {
    if (search.tab === "approvals") {
      openApprovals();
      void navigate({ search: (prev) => ({ ...prev, tab: undefined }), replace: true });
    }
  }, [search.tab, openApprovals, navigate]);

  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading apps…" />}>
      <AppsSection
        section={search.section ?? "scheduled"}
        onSection={(section: AppsSectionKind) =>
          void navigate({
            search: (prev) => ({ ...prev, section: section === "hosted" ? "hosted" : undefined }),
            replace: true,
          })
        }
        view={search.view ?? "box"}
        onView={(view) =>
          void navigate({
            search: (prev) => ({ ...prev, view: view === "table" ? "table" : undefined }),
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
    if (search.section === "hosted") {
      out.section = "hosted";
    }
    if (search.view === "table") {
      out.view = "table";
    }
    if (search.tab === "approvals") {
      out.tab = "approvals";
    }
    return out;
  },
});
