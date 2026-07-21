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
import { type ConnectionEntry, readConnections, writeConnections } from "../../lib/app-envelope";
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
  const connections = readConnections(envelope);
  const err = save.error ? toUiError(save.error) : null;

  const commit = (next: ConnectionEntry[]) => {
    save.mutate({ handle, envelope: writeConnections(envelope, next) });
  };

  return (
    <div className="skills-rail" data-testid="app-connections-rail">
      <h3>Integrations</h3>
      <p className="muted">
        Bind a connector by endpoint + the credential <em>name</em> (never the secret, D81); the
        runtime scopes the named secret onto the tool warrant at run.
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
      {err ? (
        <p className="field-error" data-testid="app-connections-error">
          {err.message}
        </p>
      ) : null}
    </div>
  );
}
