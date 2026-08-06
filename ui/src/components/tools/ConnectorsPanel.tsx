/**
 * Connectors — one surface over everything this runtime can connect to.
 *
 * This replaces two panels that described the same registry from opposite ends.
 * `IntegrationsPanel` listed the connectors that ship in the box and could only tell you
 * to go and run a CLI command; `ConnectionsPanel` listed what was actually dialed and knew
 * nothing about what shipped. Neither `Connection` nor `Integration` exists as a backend
 * type — the real axis is *catalog* (what is available) versus *instance* (what is
 * configured), which is a distinction between two STATES OF A ROW, not two destinations.
 *
 * So: one list. Every bundled connector appears whether or not it is set up, anything else
 * you have dialed appears beside them, each row says whether it is configured, and acting
 * on a row configures it — no copying a command into a terminal to get started.
 *
 * Credentials are supplied BY REFERENCE throughout: a field here takes the NAME of a
 * secret stored in Secrets, never a value, so nothing typed on this page is a credential.
 * A server needing several variables gets several environment rows.
 */

import { type FormEvent, useMemo, useState } from "react";
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
import { Badge } from "../ds/Badge";
import { GlowCard } from "../ds/GlowCard";

const TRANSPORTS = ["stdio", "http"] as const;
type Transport = (typeof TRANSPORTS)[number];

const SESSION_MODES = ["stateless", "stateful"] as const;
type SessionMode = (typeof SESSION_MODES)[number];

/** One environment entry being edited: both sides are NAMES. */
interface EnvRow {
  /** The variable the server reads. */
  name: string;
  /** The NAME of a stored secret that supplies it — never the secret. */
  credentialRef: string;
}

/**
 * The connectors shipped with this runtime.
 *
 * Static because it IS static — these are sidecar binaries the release installs beside
 * `kx`, not something discovered at runtime. Each entry is what the row needs in order to
 * configure itself, which is why the command and credential name live here rather than in
 * a tooltip telling the operator to type them somewhere else.
 */
const BUNDLED = [
  {
    name: "gmail",
    provider: "Gmail",
    command: "kx-connector-gmail",
    credentialRef: "KX_GMAIL_CREDENTIAL",
    tools: ["search", "read", "draft", "send"],
  },
  {
    name: "slack",
    provider: "Slack",
    command: "kx-connector-slack",
    credentialRef: "KX_SLACK_CREDENTIAL",
    tools: ["post_message", "read_channel", "search", "list_channels"],
  },
  {
    name: "discord",
    provider: "Discord",
    command: "kx-connector-discord",
    credentialRef: "KX_DISCORD_CREDENTIAL",
    tools: ["send_message", "read_channel", "list_channels"],
  },
  {
    name: "notion",
    provider: "Notion",
    command: "kx-connector-notion",
    credentialRef: "KX_NOTION_CREDENTIAL",
    tools: ["search", "read_page", "create_page", "append_block"],
  },
] as const;

type Bundled = (typeof BUNDLED)[number];

/** What the configure form is currently editing. */
interface Draft {
  name: string;
  transport: Transport;
  endpoint: string;
  args: string;
  tlsRequired: boolean;
  credentialRef: string;
  env: EnvRow[];
  sessionMode: SessionMode;
}

const BLANK_DRAFT: Draft = {
  name: "",
  transport: "stdio",
  endpoint: "",
  args: "",
  tlsRequired: true,
  credentialRef: "",
  env: [],
  sessionMode: "stateless",
};

function draftFor(b: Bundled): Draft {
  return {
    ...BLANK_DRAFT,
    name: b.name,
    endpoint: b.command,
    credentialRef: b.credentialRef,
  };
}

/**
 * Per-connector diagnostic: fire one of its tools and show the real result. Not a durable
 * agentic effect — the "does this actually work" check, with every state designed.
 */
function FireRow({ server }: { server: string }) {
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
        data-testid={`connector-fire-toggle-${server}`}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="chip__label">{open ? "Hide test call" : "Try a tool"}</span>
      </button>
      {open ? (
        <div className="connection-fire__form" data-testid={`connector-fire-form-${server}`}>
          <input
            type="text"
            data-testid={`connector-fire-tool-${server}`}
            placeholder="tool name (e.g. search)"
            value={tool}
            onChange={(e) => setTool(e.target.value)}
            aria-label="Tool name"
          />
          <textarea
            data-testid={`connector-fire-args-${server}`}
            placeholder={'arguments as JSON (e.g. {"query":"hello"})'}
            value={args}
            onChange={(e) => setArgs(e.target.value)}
            aria-label="Tool arguments"
            rows={2}
          />
          <button
            type="button"
            data-testid={`connector-fire-run-${server}`}
            disabled={!canFire}
            onClick={() => fire.mutate({ name: server, tool: tool.trim(), args })}
          >
            {fire.isPending ? "Running…" : "Run"}
          </button>
          {err ? (
            <p className="field-error" data-testid={`connector-fire-error-${server}`} role="alert">
              {err.message}
            </p>
          ) : result ? (
            result.ok ? (
              <pre
                className="register-tool__result mono"
                data-testid={`connector-fire-result-${server}`}
              >
                {result.resultJson}
              </pre>
            ) : (
              <p
                className="field-error"
                data-testid={`connector-fire-error-${server}`}
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

export function ConnectorsPanel() {
  const list = useListMcpServers();
  const register = useRegisterMcpServer();
  const test = useTestMcpServer();
  const discover = useDiscoverServerTools();
  const remove = useDeregisterMcpServer();

  // `null` = the form is closed. Opening it always carries a draft, so "configure this
  // connector" and "connect something else" are the same flow with different starting
  // values rather than two separate affordances.
  const [draft, setDraft] = useState<Draft | null>(null);

  const configured = useMemo(
    () => new Map(list.servers.map((s) => [s.serverName, s])),
    [list.servers],
  );
  /** Anything dialed that is not one of the bundled four. */
  const extra = useMemo(
    () => list.servers.filter((s) => !BUNDLED.some((b) => b.name === s.serverName)),
    [list.servers],
  );

  const canSubmit =
    draft !== null && draft.name.trim().length > 0 && draft.endpoint.trim().length > 0;

  const patch = (p: Partial<Draft>) => setDraft((d) => (d ? { ...d, ...p } : d));

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!draft || !canSubmit) {
      return;
    }
    register.mutate(
      {
        name: draft.name.trim(),
        transport: draft.transport,
        endpoint: draft.endpoint.trim(),
        args:
          draft.transport === "stdio" ? draft.args.split(/\s+/).filter((a) => a.length > 0) : [],
        tlsRequired: draft.transport === "http" ? draft.tlsRequired : false,
        credentialRef: draft.credentialRef.trim(),
        // Only complete rows travel: a half-typed variable would be refused by the
        // runtime, and refusing the whole form for it would lose the rest of the draft.
        env: Object.fromEntries(
          draft.env
            .filter((r) => r.name.trim().length > 0 && r.credentialRef.trim().length > 0)
            .map((r) => [r.name.trim(), r.credentialRef.trim()]),
        ),
        sessionMode: draft.sessionMode,
      },
      { onSuccess: () => setDraft(null) },
    );
  };

  const registerErr = register.error ? toUiError(register.error) : null;
  const actionError = test.error
    ? toUiError(test.error)
    : discover.error
      ? toUiError(discover.error)
      : remove.error
        ? toUiError(remove.error)
        : null;
  const actionResult = !actionError
    ? test.isSuccess
      ? test.data
        ? "Connector is reachable."
        : "Connector is unreachable — check the command or URL."
      : discover.isSuccess
        ? `Found ${discover.data.tools.length} tool(s).`
        : remove.isSuccess
          ? "Connector removed."
          : null
    : null;

  return (
    <GlowCard hover={false} variants={fadeUp} data-testid="connectors-panel">
      <h2>Connectors</h2>
      <p className="muted">
        Everything your agents can reach outside this runtime. The connectors below ship in the box;
        you can also connect any other MCP server. Each one runs as its own process — never loaded
        into the runtime — and its tools become available as{" "}
        <code className="mono">connector/tool</code>. Credentials are supplied by{" "}
        <strong>reference</strong>: you store a secret under a name in Secrets and name it here, so
        no credential is ever typed into or shown on this page.
      </p>

      {list.notWired ? (
        <EmptyState
          title="Connectors are not available on this gateway"
          detail="This runtime was built without the MCP gateway, so external servers cannot be dialed."
        />
      ) : list.isError ? (
        <ErrorNotice error={toUiError(list.error)} onRetry={() => void list.refetch()} />
      ) : list.isLoading ? (
        <EmptyState title="Loading connectors…" />
      ) : (
        <ul className="registry-list" data-testid="connectors-list">
          {BUNDLED.map((b) => (
            <ConnectorRow
              key={b.name}
              rowKey={b.name}
              title={b.name}
              provider={b.provider}
              detail={`Tools: ${b.tools.join(" · ")}`}
              server={configured.get(b.name)}
              busy={isBusy(b.name)}
              onConfigure={() => setDraft(draftFor(b))}
              onTest={() => test.mutate(b.name)}
              onDiscover={() => discover.mutate(b.name)}
              onRemove={() => remove.mutate(b.name)}
            />
          ))}
          {extra.map((s) => (
            <ConnectorRow
              key={s.connectionId}
              rowKey={s.serverName}
              title={s.serverName}
              provider={s.transport}
              detail={s.endpoint}
              server={s}
              busy={isBusy(s.serverName)}
              onTest={() => test.mutate(s.serverName)}
              onDiscover={() => discover.mutate(s.serverName)}
              onRemove={() => remove.mutate(s.serverName)}
            />
          ))}
        </ul>
      )}

      {actionError ? (
        <p className="field-error" data-testid="connector-action-error" role="alert">
          {actionError.kind === "forbidden" ? "Not permitted: " : ""}
          {actionError.message}
        </p>
      ) : actionResult ? (
        <p className="register-tool__result" data-testid="connector-action-result">
          {actionResult}
        </p>
      ) : null}

      {draft === null ? (
        <div className="chip-row">
          <button
            type="button"
            className="chip"
            data-testid="connector-add-other"
            onClick={() => setDraft(BLANK_DRAFT)}
          >
            <span className="chip__label">Connect another server</span>
          </button>
        </div>
      ) : (
        <form onSubmit={onSubmit} className="register-tool-form" data-testid="connector-form">
          <h3>{configured.has(draft.name) ? `Reconfigure ${draft.name}` : "Connect a server"}</h3>

          <fieldset className="register-tool-form__idempotency">
            <legend className="muted">How it runs</legend>
            <div className="chip-row">
              {TRANSPORTS.map((t) => (
                <button
                  key={t}
                  type="button"
                  className={`chip${draft.transport === t ? " chip--active" : ""}`}
                  data-testid={`connector-transport-${t}`}
                  aria-pressed={draft.transport === t}
                  onClick={() => patch({ transport: t })}
                >
                  <span className="chip__label">
                    {t === "stdio" ? "a local program" : "a remote URL"}
                  </span>
                </button>
              ))}
            </div>
          </fieldset>

          <div className="register-tool-form__row">
            <input
              type="text"
              data-testid="connector-name"
              placeholder="a short name (e.g. gitlab)"
              value={draft.name}
              onChange={(e) => patch({ name: e.target.value })}
              aria-label="Connector name"
            />
            <input
              type="text"
              data-testid="connector-endpoint"
              placeholder={
                draft.transport === "stdio"
                  ? "the program to run (e.g. mcp-server-gitlab)"
                  : "https://mcp.example.com/rpc"
              }
              value={draft.endpoint}
              onChange={(e) => patch({ endpoint: e.target.value })}
              aria-label={draft.transport === "stdio" ? "Program" : "URL"}
            />
          </div>

          {draft.transport === "stdio" ? (
            <input
              type="text"
              data-testid="connector-args"
              placeholder="arguments, space-separated (optional)"
              value={draft.args}
              onChange={(e) => patch({ args: e.target.value })}
              aria-label="Arguments"
            />
          ) : (
            <label className="connections-tls">
              <input
                type="checkbox"
                data-testid="connector-tls"
                checked={draft.tlsRequired}
                onChange={(e) => patch({ tlsRequired: e.target.checked })}
              />
              <span className="muted">Require HTTPS</span>
            </label>
          )}

          <input
            type="text"
            data-testid="connector-credential"
            placeholder="name of the stored secret to authenticate with (optional)"
            value={draft.credentialRef}
            onChange={(e) => patch({ credentialRef: e.target.value })}
            aria-label="Credential name"
          />

          {draft.transport === "stdio" ? (
            <fieldset className="register-tool-form__idempotency" data-testid="connector-env">
              <legend className="muted">Settings this server reads from its environment</legend>
              <p className="muted connections-session-hint">
                Some servers are configured entirely through environment variables — an API address
                alongside a token, for instance. Name the variable the server expects and the stored
                secret that supplies it. The value itself stays in Secrets.
              </p>
              {draft.env.map((row, i) => (
                // Index keys are correct here: rows are positional, edited in place, and
                // only ever removed by the button on their own line.
                // biome-ignore lint/suspicious/noArrayIndexKey: positional rows
                <div className="register-tool-form__row" key={i} data-testid={`connector-env-${i}`}>
                  <input
                    type="text"
                    data-testid={`connector-env-name-${i}`}
                    placeholder="variable name (e.g. GITLAB_API_URL)"
                    value={row.name}
                    onChange={(e) =>
                      patch({
                        env: draft.env.map((r, j) =>
                          j === i ? { ...r, name: e.target.value } : r,
                        ),
                      })
                    }
                    aria-label={`Variable name ${i + 1}`}
                  />
                  <input
                    type="text"
                    data-testid={`connector-env-ref-${i}`}
                    placeholder="stored secret name"
                    value={row.credentialRef}
                    onChange={(e) =>
                      patch({
                        env: draft.env.map((r, j) =>
                          j === i ? { ...r, credentialRef: e.target.value } : r,
                        ),
                      })
                    }
                    aria-label={`Secret name ${i + 1}`}
                  />
                  <button
                    type="button"
                    className="chip chip--danger"
                    data-testid={`connector-env-remove-${i}`}
                    onClick={() => patch({ env: draft.env.filter((_, j) => j !== i) })}
                  >
                    <span className="chip__label">Remove</span>
                  </button>
                </div>
              ))}
              <button
                type="button"
                className="chip"
                data-testid="connector-env-add"
                onClick={() => patch({ env: [...draft.env, { name: "", credentialRef: "" }] })}
              >
                <span className="chip__label">Add a setting</span>
              </button>
            </fieldset>
          ) : null}

          <fieldset className="register-tool-form__idempotency">
            <legend className="muted">Connection style</legend>
            <div className="chip-row">
              {SESSION_MODES.map((m) => (
                <button
                  key={m}
                  type="button"
                  className={`chip${draft.sessionMode === m ? " chip--active" : ""}`}
                  data-testid={`connector-session-${m}`}
                  aria-pressed={draft.sessionMode === m}
                  onClick={() => patch({ sessionMode: m })}
                >
                  <span className="chip__label">
                    {m === "stateless" ? "fresh each call" : "keep one open"}
                  </span>
                </button>
              ))}
            </div>
            <p className="muted connections-session-hint">
              Start fresh each call unless the server needs to remember what happened between calls
              — a browser session or an open transaction, say.
            </p>
          </fieldset>

          <div className="chip-row">
            <button
              type="submit"
              data-testid="connector-submit"
              disabled={register.isPending || !canSubmit}
            >
              {register.isPending ? "Connecting…" : "Connect"}
            </button>
            <button
              type="button"
              className="chip"
              data-testid="connector-cancel"
              onClick={() => setDraft(null)}
            >
              <span className="chip__label">Cancel</span>
            </button>
          </div>
        </form>
      )}

      {registerErr ? (
        <p className="field-error" data-testid="connector-error" role="alert">
          {registerErr.kind === "forbidden" ? "Not permitted: " : ""}
          {registerErr.message}
        </p>
      ) : null}
      {register.isSuccess ? (
        <p className="register-tool__result" data-testid="connector-result">
          {register.data.health === "connected"
            ? `Connected — ${register.data.discovered} tool(s) available.`
            : "Saved, but the connector could not be reached — check the settings above and try it."}
        </p>
      ) : null}

      <div className="metric-card metric-card--disabled" data-testid="connectors-cloud-disabled">
        <span className="metric-card__value">
          <span className="chip--soon">Cloud</span>
        </span>
        <span className="metric-card__label">Sign in with a provider</span>
        <span className="metric-card__sub">
          Signing in to a service directly, and sharing connectors across a team, are Cloud
          capabilities. Here, each connector uses a secret you store yourself.
        </span>
      </div>
    </GlowCard>
  );

  function isBusy(server: string): boolean {
    return (
      (test.isPending && test.variables === server) ||
      (discover.isPending && discover.variables === server) ||
      (remove.isPending && remove.variables === server)
    );
  }
}

/**
 * One row of the single list. `server` present ⇒ this connector is configured, and the row
 * shows what the runtime knows about it; absent ⇒ it is available but not set up, and the
 * only action is to configure it. The two states are deliberately the same row rather than
 * two lists, because "is this set up?" is the question the page exists to answer.
 */
function ConnectorRow({
  rowKey,
  title,
  provider,
  detail,
  server,
  busy,
  onConfigure,
  onTest,
  onDiscover,
  onRemove,
}: {
  rowKey: string;
  title: string;
  provider: string;
  detail: string;
  server?: { health: string; toolCount: number; endpoint: string; envNames: readonly string[] };
  busy: boolean;
  onConfigure?: () => void;
  onTest: () => void;
  onDiscover: () => void;
  onRemove: () => void;
}) {
  const dot = server ? healthDot(server.health) : null;
  return (
    <GlowCard className="registry-row" stripe="var(--primary)" data-testid={`connector-${rowKey}`}>
      <div className="registry-row__main">
        <div className="registry-row__head">
          {dot ? (
            <span
              className={`status-dot ${dot.cls}`}
              role="img"
              aria-label={dot.label}
              title={dot.label}
              data-testid={`connector-health-${rowKey}`}
            />
          ) : null}
          <span className="registry-row__name mono">{title}</span>
          <Badge label={provider} color="var(--primary)" />
          <Badge
            label={server ? (dot?.label ?? "set up") : "not set up"}
            color={server ? "var(--success)" : "var(--text-2)"}
          />
        </div>
        <p className="registry-row__desc muted">{server ? server.endpoint : detail}</p>
        {server ? (
          <dl className="registry-row__meta">
            <div>
              <dt className="muted">tools</dt>
              <dd className="mono">{server.toolCount}</dd>
            </div>
            {server.envNames.length > 0 ? (
              <div>
                <dt className="muted">settings</dt>
                {/* NAMES only — which variables are set, never which secret backs each. */}
                <dd className="mono" data-testid={`connector-env-names-${rowKey}`}>
                  {server.envNames.join(" · ")}
                </dd>
              </div>
            ) : null}
          </dl>
        ) : null}
        <div className="connections-list__actions chip-row">
          {server ? (
            <>
              <button
                type="button"
                className="chip"
                data-testid={`connector-test-${rowKey}`}
                disabled={busy}
                onClick={onTest}
              >
                <span className="chip__label">Check</span>
              </button>
              <button
                type="button"
                className="chip"
                data-testid={`connector-discover-${rowKey}`}
                disabled={busy}
                onClick={onDiscover}
              >
                <span className="chip__label">Refresh tools</span>
              </button>
              <button
                type="button"
                className="chip chip--danger"
                data-testid={`connector-remove-${rowKey}`}
                disabled={busy}
                onClick={onRemove}
              >
                <span className="chip__label">Remove</span>
              </button>
            </>
          ) : onConfigure ? (
            <button
              type="button"
              className="chip"
              data-testid={`connector-configure-${rowKey}`}
              onClick={onConfigure}
            >
              <span className="chip__label">Set up</span>
            </button>
          ) : null}
        </div>
        {server ? <FireRow server={rowKey} /> : null}
      </div>
    </GlowCard>
  );
}
