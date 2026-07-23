/**
 * The App INTEGRATIONS rail — bind/unbind MCP connectors on a stored App. Mirrors
 * {@link SkillsRail}: binding writes `references.connections` (+ the credential NAME
 * into `steering_config.guards.secret_scope`, never the secret — D81) and re-saves
 * via `SaveApp`. At RunApp the server resolves the connector by name + stamps the
 * scoped secret onto the tool warrant; a de-integrated fallback is refused. A LOCKED
 * App refuses the edit (disabled with the reason, D142).
 *
 * The chips + the manual bind row are {@link ConnectionsPicker} — the SAME picker the
 * New App create form mounts, so the two authoring surfaces cannot drift. This rail is
 * now just the envelope adapter over `writeConnections`.
 */

import { toUiError } from "../../kx/errors";
import { useSaveApp } from "../../kx/use-apps";
import { useListMcpServers } from "../../kx/use-connections";
import {
  type ConnectionEntry,
  readConnections,
  unbindFromSteps,
  writeConnections,
} from "../../lib/app-envelope";
import { BindingSummary } from "./BindingSummary";
import { ConnectionsPicker } from "./CapabilityPickers";

export function ConnectionsRail({
  handle,
  envelope,
  locked,
}: {
  handle: string;
  /** The stored App's parsed envelope (the `GetApp` payload). */
  envelope: Record<string, unknown>;
  locked: boolean;
}) {
  const save = useSaveApp();
  const registry = useListMcpServers();
  const connections = readConnections(envelope);
  const err = save.error ? toUiError(save.error) : null;

  const commit = (next: ConnectionEntry[]) => {
    // Detaching a connector must also scrub its binding from every step, or the blueprint
    // would name a step-binding to a `references.connections` entry that no longer exists.
    const nextDescriptors = new Set(next.map((c) => c.descriptor));
    let env = envelope;
    for (const c of connections) {
      if (!nextDescriptors.has(c.descriptor)) {
        env = unbindFromSteps(env, "connections", c.descriptor);
      }
    }
    save.mutate({ handle, envelope: writeConnections(env, next) });
  };

  /** A connector's endpoint is what the blueprint binds; show its registered name. */
  const friendly = (endpoint: string): string =>
    registry.servers.find((s) => s.endpoint === endpoint)?.serverName ?? endpoint;

  return (
    <div className="skills-rail" data-testid="app-connections-rail">
      <h3>Integrations</h3>
      <p className="muted">
        Bind a connector by endpoint + the credential <em>name</em> (never the secret, D81); the
        runtime scopes the named secret onto the tool warrant of the step(s) that use it — edit
        which on the canvas (Lineage → Edit structure).
        {locked ? " App is locked — unlock to change integrations." : ""}
      </p>
      <ConnectionsPicker
        connections={connections}
        onChange={commit}
        disabled={locked || save.isPending}
        disabledTitle={locked ? "App is locked" : "Saving…"}
        groupTestId="app-connections"
        itemTestId="app-connection"
      />
      <BindingSummary
        envelope={envelope}
        axis="connections"
        names={connections.map((c) => c.descriptor)}
        label={friendly}
        testId="app-connections-binds"
      />
      {err ? (
        <p className="field-error" data-testid="app-connections-error">
          {err.message}
        </p>
      ) : null}
    </div>
  );
}
