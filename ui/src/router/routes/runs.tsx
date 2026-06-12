import { createRoute, redirect } from "@tanstack/react-router";
import { rootRoute } from "./__root";

/**
 * PR-2 route merge (D141.1): the Workflows section lives at `/workflows`. Old
 * `/runs` and `/runs/$instanceId` deep links land there — params + search
 * (time-travel `atSeq`, the `terminal` poll-stop hint) forwarded intact.
 */
export const runsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/runs",
  beforeLoad: () => {
    throw redirect({ to: "/workflows" });
  },
});

export const runDetailRedirectRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/runs/$instanceId",
  beforeLoad: ({ params, search }) => {
    throw redirect({
      to: "/workflows/$instanceId",
      params: { instanceId: params.instanceId },
      search: search as Record<string, unknown>,
    });
  },
});
