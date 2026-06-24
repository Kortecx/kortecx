import { createRouter } from "@tanstack/react-router";
import { rootRoute } from "./routes/__root";
import { activityRoute } from "./routes/activity";
import { appsRoute } from "./routes/apps";
import { artifactsRoute } from "./routes/artifacts";
import { blueprintsNewRoute } from "./routes/blueprints-new";
import { branchesRoute } from "./routes/branches";
import { chatRoute } from "./routes/chat";
import { connectRoute } from "./routes/connect";
import { contextRoute } from "./routes/context";
import { dashboardRoute } from "./routes/dashboard";
import { datasetsRoute } from "./routes/datasets";
import { indexRoute } from "./routes/index";
import { modelsRoute } from "./routes/models";
import { monitorRoute } from "./routes/monitor";
import { recipesRoute } from "./routes/recipes";
import { runDetailRedirectRoute, runsRoute } from "./routes/runs";
import { settingsRoute } from "./routes/settings";
import { systemsRoute } from "./routes/systems";
import { toolsRoute } from "./routes/tools";
import { workflowDetailRoute } from "./routes/workflow-detail";
import { workflowsRoute } from "./routes/workflows";

const routeTree = rootRoute.addChildren([
  indexRoute,
  connectRoute,
  // PR-2 (D141.1): Workflows is the one home for run telemetry; the old
  // /runs, /runs/$id, /artifacts and /activity paths are redirect stubs.
  workflowsRoute,
  workflowDetailRoute,
  activityRoute,
  dashboardRoute,
  chatRoute,
  appsRoute,
  runsRoute,
  runDetailRedirectRoute,
  recipesRoute,
  blueprintsNewRoute,
  artifactsRoute,
  contextRoute,
  branchesRoute,
  datasetsRoute,
  toolsRoute,
  modelsRoute,
  monitorRoute,
  systemsRoute,
  settingsRoute,
]);

export const router = createRouter({
  routeTree,
  defaultPreload: "intent",
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
