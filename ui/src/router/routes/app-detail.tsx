import { createRoute, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import type { IdeTab } from "../../components/sections/AppDetailSection";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const ROUTE_ID = "/apps/$handle";
const IDE_TABS = ["files", "lineage", "skills", "capabilities"] as const;

/** POC-5d: the App IDE's URL state — the active tab + the selected file path are
 *  addressable so refresh / back-forward / deep links survive. */
interface AppDetailSearch {
  tab?: IdeTab;
  path?: string;
}

const AppDetailSection = lazy(() =>
  import("../../components/sections/AppDetailSection").then((m) => ({
    default: m.AppDetailSection,
  })),
);

function AppDetailScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return <AppDetailContent />;
}

function AppDetailContent() {
  const { handle } = useParams({ from: ROUTE_ID });
  const search = useSearch({ from: ROUTE_ID });
  const navigate = useNavigate({ from: ROUTE_ID });
  return (
    <Suspense fallback={<EmptyState title="Loading app…" />}>
      <AppDetailSection
        handle={handle}
        tab={search.tab ?? "files"}
        path={search.path}
        onTab={(tab) =>
          // Leaving Files drops the file deep link.
          void navigate({
            search: (prev) => ({
              ...prev,
              tab: tab === "files" ? undefined : tab,
              path: tab === "files" ? prev.path : undefined,
            }),
          })
        }
        onPath={(path) => void navigate({ search: (prev) => ({ ...prev, path }) })}
      />
    </Suspense>
  );
}

export const appDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: ROUTE_ID,
  validateSearch: (search: Record<string, unknown>): AppDetailSearch => {
    const out: AppDetailSearch = {};
    if (typeof search.tab === "string" && (IDE_TABS as readonly string[]).includes(search.tab)) {
      out.tab = search.tab as IdeTab;
    }
    if (typeof search.path === "string" && search.path.length > 0) {
      out.path = search.path;
    }
    return out;
  },
  component: AppDetailScreen,
});
