/**
 * The App INTEGRATIONS rail — bind/unbind MCP connectors on a stored App. Mirrors
 * {@link SkillsRail}: binding writes `references.connections` (+ the credential NAME
 * into `steering_config.guards.secret_scope`, never the secret — D81) and re-saves
 * via `SaveApp`. At RunApp the server resolves the connector by name + stamps the
 * scoped secret onto the tool warrant; a de-integrated fallback is refused. A LOCKED
 * App refuses the edit (disabled with the reason, D142). Registered servers offer a
 * one-click bind (their endpoint IS the descriptor); a manual endpoint + credential
 * name covers anything not yet registered. `ListMcpServers` returns only whether a
 * credential is present (a boolean, D81), so the NAME is always a text field.
 */

import { useState } from "react";
import { toUiError } from "../../kx/errors";
import { useSaveApp } from "../../kx/use-apps";
import { useListMcpServers } from "../../kx/use-connections";
import { type ConnectionEntry, readConnections, writeConnections } from "../../lib/app-envelope";

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
  const registry = useListMcpServers();
  const save = useSaveApp();
  const connections = readConnections(envelope);
  const [descriptor, setDescriptor] = useState("");
  const [credential, setCredential] = useState("");
  const err = save.error ? toUiError(save.error) : null;
  const boundDescriptors = new Set(connections.map((c) => c.descriptor));
  const available = registry.servers.filter((s) => !boundDescriptors.has(s.endpoint));
  const disabled = locked || save.isPending;

  const commit = (next: ConnectionEntry[]) => {
    save.mutate({ handle, envelope: writeConnections(envelope, next) });
  };
  const bind = (desc: string, cred: string) => {
    const d = desc.trim();
    if (d === "" || boundDescriptors.has(d)) {
      return;
    }
    commit([...connections, { descriptor: d, credential_ref: cred.trim() }]);
    setDescriptor("");
    setCredential("");
  };
  const unbind = (desc: string) => commit(connections.filter((c) => c.descriptor !== desc));

  return (
    <div className="skills-rail" data-testid="app-connections-rail">
      <h3>Integrations</h3>
      <p className="muted">
        Bind a connector by endpoint + the credential <em>name</em> (never the secret, D81); the
        runtime scopes the named secret onto the tool warrant at run.
        {locked ? " App is locked — unlock to change integrations." : ""}
      </p>
      <div className="chip-row" data-testid="app-connections-attached">
        {connections.length === 0 ? (
          <span className="muted">No integrations bound.</span>
        ) : (
          connections.map((c) => (
            <button
              key={c.descriptor}
              type="button"
              className="chip chip--active"
              disabled={disabled}
              title={locked ? "App is locked" : `Unbind ${c.descriptor}`}
              onClick={() => unbind(c.descriptor)}
              data-testid={`app-connection-detach-${c.descriptor}`}
            >
              {c.descriptor}
              {c.credential_ref ? ` · ${c.credential_ref}` : ""} ✕
            </button>
          ))
        )}
      </div>
      {available.length > 0 ? (
        <div className="chip-row" data-testid="app-connections-available">
          {available.map((s) => (
            <button
              key={s.connectionId}
              type="button"
              className="chip"
              disabled={disabled}
              title={locked ? "App is locked" : `Bind ${s.serverName} (${s.endpoint})`}
              onClick={() => bind(s.endpoint, "")}
              data-testid={`app-connection-add-${s.serverName}`}
            >
              + {s.serverName}
            </button>
          ))}
        </div>
      ) : null}
      <div className="chip-row">
        <input
          className="input"
          placeholder="endpoint (stdio command or https URL)"
          value={descriptor}
          disabled={disabled}
          onChange={(e) => setDescriptor(e.target.value)}
          data-testid="app-connection-descriptor"
          aria-label="Connector endpoint"
          spellCheck={false}
          autoComplete="off"
        />
        <input
          className="input"
          placeholder="credential name (optional)"
          value={credential}
          disabled={disabled}
          onChange={(e) => setCredential(e.target.value)}
          data-testid="app-connection-credential"
          aria-label="Credential name"
          spellCheck={false}
          autoComplete="off"
        />
        <button
          type="button"
          className="btn-ghost"
          disabled={disabled || descriptor.trim() === ""}
          onClick={() => bind(descriptor, credential)}
          data-testid="app-connection-bind"
        >
          Bind
        </button>
      </div>
      {err ? (
        <p className="field-error" data-testid="app-connections-error">
          {err.message}
        </p>
      ) : null}
    </div>
  );
}
