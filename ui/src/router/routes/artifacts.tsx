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
  /** Gallery mode: browse all of this run's committed artifacts. */
  run?: string;
  /** Deep-link mode: one committed artifact (`instance` + `ref`). */
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
  const { run, instance, ref } = useSearch({ from: ROUTE_ID });
  return (
    <Suspense fallback={<EmptyState title="Loading…" />}>
      <ArtifactsSection runId={run} instanceId={instance} contentRef={ref} />
    </Suspense>
  );
}

export const artifactsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: ROUTE_ID,
  validateSearch: (search: Record<string, unknown>): ArtifactsSearch => {
    const out: ArtifactsSearch = {};
    // A run instance id is a 16-byte (32 hex char) server-derived id.
    if (typeof search.run === "string" && /^[0-9a-f]{32}$/.test(search.run)) {
      out.run = search.run;
    }
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
