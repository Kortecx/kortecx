import { createRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const ROUTE_ID = "/activity";

// Code-split the dashboard (pulls metrics/feed/scrubber) into its own chunk.
const ActivityPanel = lazy(() =>
  import("../../components/activity/ActivityPanel").then((m) => ({ default: m.ActivityPanel })),
);

interface ActivitySearch {
  instance?: string;
  atSeq?: number;
}

function ActivityScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return <ActivityRouter />;
}

function ActivityRouter() {
  const navigate = useNavigate();
  const { instance, atSeq } = useSearch({ from: ROUTE_ID });
  return (
    <Suspense fallback={<EmptyState title="Loading…" />}>
      <ActivityPanel
        instance={instance}
        atSeq={atSeq}
        onSelectInstance={(id) => navigate({ to: ROUTE_ID, search: { instance: id } })}
        onAtSeq={(seq) =>
          navigate({
            to: ROUTE_ID,
            search: seq != null ? { instance, atSeq: seq } : { instance },
          })
        }
      />
    </Suspense>
  );
}

export const activityRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: ROUTE_ID,
  validateSearch: (search: Record<string, unknown>): ActivitySearch => {
    const out: ActivitySearch = {};
    if (typeof search.instance === "string" && /^[0-9a-f]{32}$/.test(search.instance)) {
      out.instance = search.instance;
    }
    const raw = search.atSeq;
    const n = typeof raw === "string" ? Number(raw) : typeof raw === "number" ? raw : Number.NaN;
    if (Number.isFinite(n) && n >= 0) {
      out.atSeq = Math.floor(n);
    }
    return out;
  },
  component: ActivityScreen,
});
