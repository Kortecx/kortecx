import { Navigate, createRoute } from "@tanstack/react-router";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

function IndexRedirect() {
  const { status } = useConnection();
  return <Navigate to={status === "connected" ? "/activity" : "/connect"} />;
}

export const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: IndexRedirect,
});
