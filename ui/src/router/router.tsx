import { createRouter } from "@tanstack/react-router";
import { rootRoute } from "./routes/__root";
import { activityRoute } from "./routes/activity";
import { artifactsRoute } from "./routes/artifacts";
import { chatRoute } from "./routes/chat";
import { connectRoute } from "./routes/connect";
import { contextRoute } from "./routes/context";
import { datasetsRoute } from "./routes/datasets";
import { indexRoute } from "./routes/index";
import { monitorRoute } from "./routes/monitor";
import { recipesRoute } from "./routes/recipes";
import { runDetailRoute } from "./routes/run-detail";
import { runsRoute } from "./routes/runs";
import { settingsRoute } from "./routes/settings";
import { systemsRoute } from "./routes/systems";
import { toolsRoute } from "./routes/tools";

const routeTree = rootRoute.addChildren([
  indexRoute,
  connectRoute,
  activityRoute,
  chatRoute,
  runsRoute,
  runDetailRoute,
  recipesRoute,
  artifactsRoute,
  contextRoute,
  datasetsRoute,
  toolsRoute,
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
