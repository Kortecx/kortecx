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
  useCallMcpTool,
  useDeregisterMcpServer,
  useDiscoverServerTools,
  useListMcpServers,
  useRegisterMcpServer,
  useTestMcpServer,
} from "../../kx/use-connections";
import { healthDot } from "../../lib/connection-health";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { GlowCard } from "../ds/GlowCard";

const TRANSPORTS = ["stdio", "http"] as const;
type Transport = (typeof TRANSPORTS)[number];

const SESSION_MODES = ["stateless", "stateful"] as const;
type SessionMode = (typeof SESSION_MODES)[number];

/**
 * Per-server operator DIAGNOSTIC: fire ONE registered tool live through the broker
 * (`CallMcpTool`) and show the real result. NOT a durable agentic effect (the agentic
 * loop fires the same tools durably) — the "does this connector actually work" check.
 * Every state designed (D142): idle / firing / ok / error; encapsulates its own state
 * so each row is independent.
 */
function ConnectionFireRow({ server }: { server: string }) {
  const fire = useCallMcpTool();
  const [open, setOpen] = useState(false);
  const [tool, setTool] = useState("");
  const [args, setArgs] = useState("{}");
  const canFire = tool.trim().length > 0 && !fire.isPending;
  const result = fire.data;
  const err = fire.error ? toUiError(fire.error) : null;

  return (
    <div className="connection-fire">
      <button
        type="button"
        className="chip"
        data-testid={`connection-fire-toggle-${server}`}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="chip__label">{open ? "Hide fire" : "Fire a tool"}</span>
      </button>
      {open ? (
        <div className="connection-fire__form" data-testid={`connection-fire-form-${server}`}>
          <input
            type="text"
            data-testid={`connection-fire-tool-${server}`}
            placeholder="tool remote name (e.g. reverse)"
            value={tool}
            onChange={(e) => setTool(e.target.value)}
            aria-label="Tool remote name"
          />
          <textarea
            data-testid={`connection-fire-args-${server}`}
            placeholder={'args JSON (e.g. {"text":"hi"})'}
            value={args}
            onChange={(e) => setArgs(e.target.value)}
            aria-label="Tool arguments JSON"
            rows={2}
          />
          <button
            type="button"
            data-testid={`connection-fire-run-${server}`}
            disabled={!canFire}
            onClick={() => fire.mutate({ name: server, tool: tool.trim(), args })}
          >
            {fire.isPending ? "Firing…" : "Fire"}
          </button>
          {err ? (
            <p className="field-error" data-testid={`connection-fire-error-${server}`} role="alert">
              {err.message}
            </p>
          ) : result ? (
            result.ok ? (
              <pre
                className="register-tool__result mono"
                data-testid={`connection-fire-result-${server}`}
              >
                {result.resultJson}
              </pre>
            ) : (
              <p
                className="field-error"
                data-testid={`connection-fire-error-${server}`}
                role="alert"
              >
                {result.error}
              </p>
            )
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/** G1: curated first-class Integration providers — a one-click prefill of the "Add a
 * server" form (over the EXISTING RegisterMcpServer + PutSecret RPCs; no new proto).
 * Mirrors the CLI `kx connections add --provider gmail` + the SDK provider catalog. */
const PROVIDERS = [
  {
    id: "gmail",
    label: "Gmail",
    command: "kx-connector-gmail",
    credentialRef: "KX_GMAIL_CREDENTIAL",
  },
  {
    id: "discord",
    label: "Discord",
    command: "kx-connector-discord",
    credentialRef: "KX_DISCORD_CREDENTIAL",
  },
  {
    id: "slack",
    label: "Slack",
    command: "kx-connector-slack",
    credentialRef: "KX_SLACK_CREDENTIAL",
  },
  {
    id: "notion",
    label: "Notion",
    command: "kx-connector-notion",
    credentialRef: "KX_NOTION_CREDENTIAL",
  },
] as const;

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
  // PR-6b-3: the firing posture. Stateless-first (the default); stateful reuses
  // one long-lived session for servers that require it.
  const [sessionMode, setSessionMode] = useState<SessionMode>("stateless");

  const canSubmit = name.trim().length > 0 && endpoint.trim().length > 0;

  // G1: prefill the form from a curated provider (the operator reviews + submits, then
  // sets its credential secret in the Secrets panel). No hidden authority — the same
  // RegisterMcpServer the manual form uses.
  const connectProvider = (p: (typeof PROVIDERS)[number]) => {
    setName(p.id);
    setTransport("stdio");
    setEndpoint(p.command);
    setCredentialRef(p.credentialRef);
    setSessionMode("stateless");
  };

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
        sessionMode,
      },
      {
        onSuccess: () => {
          setName("");
          setEndpoint("");
          setArgs("");
          setCredentialRef("");
          setSessionMode("stateless");
        },
      },
    );
  };

  const registerErr = register.error ? toUiError(register.error) : null;

  // The most recent per-row action's error (Test / Re-discover / Remove) — surfaced
  // so NotFound / RateLimited / Dial refusals are never swallowed (review #3).
  const actionError = test.error
    ? toUiError(test.error)
    : discover.error
      ? toUiError(discover.error)
      : remove.error
        ? toUiError(remove.error)
        : null;
  // ...or its transient success outcome.
  const actionResult = !actionError
    ? test.isSuccess
      ? test.data
        ? "Server is reachable."
        : "Server is unreachable — check the endpoint."
      : discover.isSuccess
        ? `Re-discovered ${discover.data.tools.length} tool(s).`
        : remove.isSuccess
          ? "Server removed."
          : null
    : null;

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
                  {s.sessionMode === "stateful" ? (
                    <span
                      className="chip chip--static"
                      data-testid={`connection-session-${s.serverName}`}
                      title="Reuses one long-lived session across calls"
                    >
                      <span className="chip__label">stateful</span>
                    </span>
                  ) : null}
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
                <ConnectionFireRow server={s.serverName} />
              </li>
            );
          })}
        </ul>
      )}

      {/* Per-action outcome + error (Test / Re-discover / Remove). D142: every
          state designed — these mutations would otherwise be silent (review #3). */}
      {actionError ? (
        <p className="field-error" data-testid="connection-action-error" role="alert">
          {actionError.kind === "forbidden" ? "Not permitted: " : ""}
          {actionError.message}
        </p>
      ) : actionResult ? (
        <p className="register-tool__result" data-testid="connection-action-result">
          {actionResult}
        </p>
      ) : null}

      <form onSubmit={onSubmit} className="register-tool-form" data-testid="connections-add-form">
        <h3>Add a server</h3>
        <fieldset className="register-tool-form__idempotency">
          <legend className="muted">Connect an integration</legend>
          <div className="chip-row">
            {PROVIDERS.map((p) => (
              <button
                key={p.id}
                type="button"
                className="chip"
                data-testid={`connection-provider-${p.id}`}
                title={`Prefill the form for the bundled ${p.label} connector (${p.command}); set ${p.credentialRef} in Secrets. An agent grants its tools as "${p.id}/<tool>" (the connection name), e.g. ${p.id}/read.`}
                onClick={() => connectProvider(p)}
              >
                <span className="chip__label">Connect {p.label}</span>
              </button>
            ))}
          </div>
        </fieldset>
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
        <fieldset className="register-tool-form__idempotency">
          <legend className="muted">Session</legend>
          <div className="chip-row">
            {SESSION_MODES.map((m) => (
              <button
                key={m}
                type="button"
                className={`chip${sessionMode === m ? " chip--active" : ""}`}
                data-testid={`connection-session-mode-${m}`}
                aria-pressed={sessionMode === m}
                onClick={() => setSessionMode(m)}
              >
                <span className="chip__label">{m}</span>
              </button>
            ))}
          </div>
          <p className="muted connections-session-hint">
            Stateless fires each call as a fresh single-shot session (best for read tools &amp;
            load-balanced servers). Stateful reuses one live session — for servers that require it.
          </p>
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
