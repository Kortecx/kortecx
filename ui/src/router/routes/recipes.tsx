import { createRoute, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const RecipesSection = lazy(() =>
  import("../../components/sections/RecipesSection").then((m) => ({ default: m.RecipesSection })),
);

/** The clone-lite landing (PR-2.1): `?handle=` preselects a blueprint and
 *  `?args=` (JSON text) prefills its form — how a Workflows row's "Clone"
 *  opens a run's blueprint with its prior inputs ready to tweak. */
interface RecipesSearch {
  handle?: string;
  args?: string;
}

function RecipesScreen() {
  const { status } = useConnection();
  const { handle, args } = useSearch({ from: "/recipes" });
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading blueprints…" />}>
      <RecipesSection initialHandle={handle} initialArgs={args} />
    </Suspense>
  );
}

export const recipesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/recipes",
  validateSearch: (search: Record<string, unknown>): RecipesSearch => {
    const out: RecipesSearch = {};
    // A recipe handle is "namespace/collection/name" — short, slash-separated.
    if (
      typeof search.handle === "string" &&
      search.handle.length <= 200 &&
      /^[\w./-]+$/.test(search.handle)
    ) {
      out.handle = search.handle;
    }
    // Prefill args must be a small JSON OBJECT (the server re-validates at
    // bind anyway — this is a display-side sanity cap, fail-closed).
    if (typeof search.args === "string" && search.args.length <= 8192) {
      try {
        const parsed: unknown = JSON.parse(search.args);
        if (parsed !== null && typeof parsed === "object" && !Array.isArray(parsed)) {
          out.args = search.args;
        }
      } catch {
        /* drop unparseable prefill */
      }
    }
    return out;
  },
  component: RecipesScreen,
});
