import { createRoute, useNavigate } from "@tanstack/react-router";
import { ConnectionForm } from "../../components/ConnectionForm";
import { ErrorNotice } from "../../components/ErrorNotice";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

function ConnectScreen() {
  const { status, endpoint, wsEndpoint, error, connect } = useConnection();
  const navigate = useNavigate();

  return (
    <section className="screen">
      <h1>Connect to a gateway</h1>
      <p className="muted">
        Point at a running <code>kx serve</code>. The bearer token is kept in memory only — never
        stored.
      </p>
      {error ? <ErrorNotice error={error} /> : null}
      <ConnectionForm
        initialEndpoint={endpoint}
        initialWsEndpoint={wsEndpoint ?? ""}
        connecting={status === "connecting"}
        onConnect={async (ep, token, ws) => {
          const ok = await connect(ep, token, ws);
          if (ok) {
            navigate({ to: "/chat" });
          }
        }}
      />
    </section>
  );
}

export const connectRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/connect",
  component: ConnectScreen,
});
