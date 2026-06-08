import { createRoute, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const ROUTE_ID = "/artifacts";

const ArtifactsSection = lazy(() =>
  import("../../components/sections/ArtifactsSection").then((m) => ({
    default: m.ArtifactsSection,
  })),
);

interface ArtifactsSearch {
  instance?: string;
  ref?: string;
}

function ArtifactsScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return <ArtifactsRouter />;
}

function ArtifactsRouter() {
  const { instance, ref } = useSearch({ from: ROUTE_ID });
  return (
    <Suspense fallback={<EmptyState title="Loading…" />}>
      <ArtifactsSection instanceId={instance} contentRef={ref} />
    </Suspense>
  );
}

export const artifactsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: ROUTE_ID,
  validateSearch: (search: Record<string, unknown>): ArtifactsSearch => {
    const out: ArtifactsSearch = {};
    if (typeof search.instance === "string" && /^[0-9a-f]{32}$/.test(search.instance)) {
      out.instance = search.instance;
    }
    // A content ref is a 32-byte (64 hex char) server-derived id.
    if (typeof search.ref === "string" && /^[0-9a-f]{64}$/.test(search.ref)) {
      out.ref = search.ref;
    }
    return out;
  },
  component: ArtifactsScreen,
});
