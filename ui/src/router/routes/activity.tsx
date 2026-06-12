import { createRoute, redirect } from "@tanstack/react-router";
import { rootRoute } from "./__root";

/**
 * Activity is no longer a sidebar section: the run-scoped feed/metrics/time-travel
 * panel now lives in the navbar's ACTIVITY DRAWER (next to the devtools + refresh
 * controls — the spec's top-bar activities affordance). Old deep links land on
 * Workflows; the drawer is one click away from anywhere.
 */
export const activityRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/activity",
  beforeLoad: () => {
    throw redirect({ to: "/workflows" });
  },
});
