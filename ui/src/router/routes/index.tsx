import { Navigate, createRoute } from "@tanstack/react-router";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

function IndexRedirect() {
  const { status } = useConnection();
  // Chat is the connected default (D137); disconnected lands on the login gate.
  return <Navigate to={status === "connected" ? "/chat" : "/connect"} />;
}

export const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: IndexRedirect,
});
