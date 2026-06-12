import { createRoute } from "@tanstack/react-router";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

/**
 * Context — reusable instruction & file bundles (the spec's eighth-IA section).
 * The gateway exposes NO context-bundle surface yet (the bundle store + the
 * attach-to-invoke wire land with the Context backend), so this section renders
 * an HONEST not-wired state — never a mock (don't-fake-gaps).
 */
function ContextScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <section className="screen" data-testid="context-section">
      <h1>Context</h1>
      <p className="muted">
        Reusable instruction &amp; file bundles you can attach to chats and workflows.
      </p>
      <EmptyState
        title="No context-bundle surface on this gateway yet"
        detail="Bundles (skills, instructions, files) become attachable once the gateway ships its context backend — nothing is faked in the meantime."
      />
    </section>
  );
}

export const contextRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/context",
  component: ContextScreen,
});
