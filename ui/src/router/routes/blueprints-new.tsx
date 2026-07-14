import { createRoute, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { ErrorNotice } from "../../components/ErrorNotice";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { useCloneGraph } from "../../kx/use-clone-graph";
import { rootRoute } from "./__root";

const BlueprintBuilderSection = lazy(() =>
  import("../../components/sections/BlueprintBuilderSection").then((m) => ({
    default: m.BlueprintBuilderSection,
  })),
);

/** `?clone=<instanceId>` (32-hex) seeds the builder from a committed run's DAG
 *  (clone-to-edit, D141.4); `?app=<handle>` seeds it from a saved App's blueprint to
 *  EDIT the App's structure (POC-5d); absent ⇒ a fresh blueprint. `app` wins over
 *  `clone` when both are present. */
interface BuilderSearch {
  clone?: string;
  app?: string;
}

/** Loads + reconstructs a run's DAG, then opens the builder seeded from it. */
function CloneLoader({ instanceId }: { instanceId: string }) {
  const { graph, loading, error } = useCloneGraph(instanceId);
  if (loading) {
    return <EmptyState title="Cloning workflow…" detail="Reconstructing the graph to remix." />;
  }
  if (error) {
    return <ErrorNotice error={toUiError(error)} />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading builder…" />}>
      <BlueprintBuilderSection initialGraph={graph} />
    </Suspense>
  );
}

function BuilderScreen() {
  const { status } = useConnection();
  const { clone, app } = useSearch({ from: "/blueprints/new" });
  if (status !== "connected") {
    return <ConnectGate />;
  }
  // app-edit: the lazy builder fetches + parses the App itself (App hooks + parser stay
  // out of this eager route module); the route only forwards the handle.
  if (app) {
    return (
      <Suspense fallback={<EmptyState title="Loading builder…" />}>
        <BlueprintBuilderSection mode={{ kind: "app-edit", handle: app }} />
      </Suspense>
    );
  }
  if (clone) {
    return <CloneLoader instanceId={clone} />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading builder…" />}>
      <BlueprintBuilderSection />
    </Suspense>
  );
}

export const blueprintsNewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/blueprints/new",
  validateSearch: (search: Record<string, unknown>): BuilderSearch => {
    const out: BuilderSearch = {};
    // An instance id is a 16-byte (32 hex char) server-derived id.
    if (typeof search.clone === "string" && /^[0-9a-f]{32}$/.test(search.clone)) {
      out.clone = search.clone;
    }
    // An App handle (e.g. `kx/apps/<slug>`) — a conservative safe charset; the server
    // re-validates ownership (a not-owned handle returns a uniform not-found).
    if (typeof search.app === "string" && /^[A-Za-z0-9][A-Za-z0-9/_.-]{0,200}$/.test(search.app)) {
      out.app = search.app;
    }
    return out;
  },
  component: BuilderScreen,
});
