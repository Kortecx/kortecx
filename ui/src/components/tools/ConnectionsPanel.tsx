/**
 * The live MCP Connections panel (PR-6b-1) — the govern surface over the external
 * MCP gateway. Replaces the PR-6a-2 honest-disabled `ConnectionsCard`.
 *
 * Register + DIAL an external MCP server (stdio command or HTTP URL, incl.
 * Py/TS-SDK-exposed gateways); list servers with their folded health + discovered
 * tool count; test reachability; re-discover; remove. The runtime is a SECURE
 * GATEWAY (D132/D159/GR19): the host is SSRF-vetted at admission AND at dial; a
 * credential is referenced by NAME only (never the secret, D81); ids are
 * server-derived (SN-8). OAuth/device-flow + a credential marketplace are CLOUD —
 * shown as an honest-disabled affordance (GR15 don't-fake-gaps).
 *
 * Transport is chosen via CHIP buttons (never a controlled `<select>` — the UI-3
 * React-controlled-select e2e gotcha). Degrades to a not-wired state on a gateway
 * without the MCP gateway feature (UNIMPLEMENTED).
 */

import { type FormEvent, useState } from "react";
import { fadeUp } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import {
  useDeregisterMcpServer,
  useDiscoverServerTools,
  useListMcpServers,
  useRegisterMcpServer,
  useTestMcpServer,
} from "../../kx/use-connections";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { GlowCard } from "../ds/GlowCard";

const TRANSPORTS = ["stdio", "http"] as const;
type Transport = (typeof TRANSPORTS)[number];

/** Map a folded health tag to an existing status-dot modifier + an a11y label. */
function healthDot(health: string): { cls: string; label: string } {
  switch (health) {
    case "connected":
      return { cls: "status-dot--online", label: "connected" };
    case "unreachable":
      return { cls: "status-dot--error", label: "unreachable" };
    default:
      return { cls: "status-dot--offline", label: "not dialed" };
  }
}

export function ConnectionsPanel() {
  const list = useListMcpServers();
  const register = useRegisterMcpServer();
  const test = useTestMcpServer();
  const discover = useDiscoverServerTools();
  const remove = useDeregisterMcpServer();

  const [name, setName] = useState("");
  const [transport, setTransport] = useState<Transport>("stdio");
  const [endpoint, setEndpoint] = useState("");
  const [args, setArgs] = useState("");
  const [tlsRequired, setTlsRequired] = useState(true);
  const [credentialRef, setCredentialRef] = useState("");

  const canSubmit = name.trim().length > 0 && endpoint.trim().length > 0;

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) {
      return;
    }
    register.mutate(
      {
        name: name.trim(),
        transport,
        endpoint: endpoint.trim(),
        args: transport === "stdio" ? args.split(/\s+/).filter((a) => a.length > 0) : [],
        tlsRequired: transport === "http" ? tlsRequired : false,
        credentialRef: credentialRef.trim(),
      },
      {
        onSuccess: () => {
          setName("");
          setEndpoint("");
          setArgs("");
          setCredentialRef("");
        },
      },
    );
  };

  const registerErr = register.error ? toUiError(register.error) : null;

  return (
    <GlowCard hover={false} variants={fadeUp} data-testid="connections-panel">
      <h2>MCP Connections</h2>
      <p className="muted">
        Dial external MCP servers (stdio · HTTP, incl. Py/TS-SDK-exposed gateways). Registering
        DIALS the server and registers its tools (namespaced{" "}
        <code className="mono">server/tool</code>). The host is SSRF-vetted; a credential is
        referenced by NAME only (an env var / vault key — never the secret).
      </p>

      {list.notWired ? (
        <EmptyState
          title="MCP gateway not enabled"
          detail="Run a gateway built with the mcp-gateway feature (on by default) to dial external MCP servers."
        />
      ) : list.isError ? (
        <ErrorNotice error={toUiError(list.error)} />
      ) : list.isLoading ? (
        <EmptyState title="Loading connections…" />
      ) : list.servers.length === 0 ? (
        <EmptyState
          title="No MCP servers connected"
          detail="Add a server below to dial it and discover its tools."
        />
      ) : (
        <ul className="connections-list" data-testid="connections-list">
          {list.servers.map((s) => {
            const dot = healthDot(s.health);
            const busy =
              (test.isPending && test.variables === s.serverName) ||
              (discover.isPending && discover.variables === s.serverName) ||
              (remove.isPending && remove.variables === s.serverName);
            return (
              <li
                key={s.connectionId}
                className="connections-list__row"
                data-testid={`connection-${s.serverName}`}
              >
                <div className="connections-list__head">
                  <span
                    className={`status-dot ${dot.cls}`}
                    role="img"
                    aria-label={dot.label}
                    title={dot.label}
                  />
                  <span className="connections-list__name">{s.serverName}</span>
                  <span className="chip chip--static">
                    <span className="chip__label">{s.transport}</span>
                  </span>
                  {s.credentialRefPresent ? (
                    <span className="chip chip--static" title="A credential ref name is attached">
                      <span className="chip__label">cred</span>
                    </span>
                  ) : null}
                </div>
                <div className="connections-list__meta muted">
                  <code className="mono">{s.endpoint}</code>
                  <span>· {s.toolCount} tool(s)</span>
                  <span>· {dot.label}</span>
                </div>
                <div className="connections-list__actions chip-row">
                  <button
                    type="button"
                    className="chip"
                    data-testid={`connection-test-${s.serverName}`}
                    disabled={busy}
                    onClick={() => test.mutate(s.serverName)}
                  >
                    <span className="chip__label">Test</span>
                  </button>
                  <button
                    type="button"
                    className="chip"
                    data-testid={`connection-discover-${s.serverName}`}
                    disabled={busy}
                    onClick={() => discover.mutate(s.serverName)}
                  >
                    <span className="chip__label">Re-discover</span>
                  </button>
                  <button
                    type="button"
                    className="chip chip--danger"
                    data-testid={`connection-remove-${s.serverName}`}
                    disabled={busy}
                    onClick={() => remove.mutate(s.serverName)}
                  >
                    <span className="chip__label">Remove</span>
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}

      <form onSubmit={onSubmit} className="register-tool-form" data-testid="connections-add-form">
        <h3>Add a server</h3>
        <fieldset className="register-tool-form__idempotency">
          <legend className="muted">Transport</legend>
          <div className="chip-row">
            {TRANSPORTS.map((t) => (
              <button
                key={t}
                type="button"
                className={`chip${transport === t ? " chip--active" : ""}`}
                data-testid={`connection-transport-${t}`}
                aria-pressed={transport === t}
                onClick={() => setTransport(t)}
              >
                <span className="chip__label">{t}</span>
              </button>
            ))}
          </div>
        </fieldset>
        <div className="register-tool-form__row">
          <input
            type="text"
            data-testid="connection-name"
            placeholder="server name (e.g. github)"
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label="Server name"
          />
          <input
            type="text"
            data-testid="connection-endpoint"
            placeholder={
              transport === "stdio"
                ? "command path (e.g. my-mcp-server)"
                : "https://mcp.example.com/rpc"
            }
            value={endpoint}
            onChange={(e) => setEndpoint(e.target.value)}
            aria-label={transport === "stdio" ? "Command path" : "Endpoint URL"}
          />
        </div>
        {transport === "stdio" ? (
          <input
            type="text"
            data-testid="connection-args"
            placeholder="args (space-separated, optional)"
            value={args}
            onChange={(e) => setArgs(e.target.value)}
            aria-label="Command arguments"
          />
        ) : (
          <label className="connections-tls">
            <input
              type="checkbox"
              data-testid="connection-tls"
              checked={tlsRequired}
              onChange={(e) => setTlsRequired(e.target.checked)}
            />
            <span className="muted">Require TLS (refuse plaintext http://)</span>
          </label>
        )}
        <input
          type="text"
          data-testid="connection-credential"
          placeholder="credential ref name (env var / vault key, optional)"
          value={credentialRef}
          onChange={(e) => setCredentialRef(e.target.value)}
          aria-label="Credential reference name"
        />
        <button
          type="submit"
          data-testid="connection-add-submit"
          disabled={register.isPending || !canSubmit}
        >
          {register.isPending ? "Dialing…" : "Add & dial server"}
        </button>
      </form>

      {registerErr ? (
        <p className="field-error" data-testid="connection-add-error" role="alert">
          {registerErr.kind === "forbidden" ? "Host not permitted: " : ""}
          {registerErr.message}
        </p>
      ) : null}
      {register.isSuccess ? (
        <p className="register-tool__result" data-testid="connection-add-result">
          {register.data.health === "connected"
            ? `Connected — ${register.data.discovered} tool(s) discovered.`
            : "Registered, but the server is unreachable — check the endpoint and Test it."}
        </p>
      ) : null}

      <div className="metric-card metric-card--disabled" data-testid="connections-cloud-disabled">
        <span className="metric-card__value">
          <span className="chip--soon">Cloud</span>
        </span>
        <span className="metric-card__label">OAuth & credential marketplace</span>
        <span className="metric-card__sub">
          Interactive OAuth / device-flow setup and a hosted, multi-tenant credential marketplace
          are a Cloud capability. OSS uses secret-less credential references (env var / vault key).
        </span>
      </div>
    </GlowCard>
  );
}
