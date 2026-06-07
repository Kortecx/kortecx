import { createRouter } from "@tanstack/react-router";
import { rootRoute } from "./routes/__root";
import { connectRoute } from "./routes/connect";
import { indexRoute } from "./routes/index";
import { runDetailRoute } from "./routes/run-detail";
import { runsRoute } from "./routes/runs";

const routeTree = rootRoute.addChildren([indexRoute, connectRoute, runsRoute, runDetailRoute]);

export const router = createRouter({
  routeTree,
  defaultPreload: "intent",
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
