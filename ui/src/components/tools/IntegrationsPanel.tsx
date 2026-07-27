/**
 * The connectors bundled with this runtime, and whether each is actually dialed.
 *
 * These ship in the box but had no surface: an operator could only discover them
 * by reading the repository, and nothing showed whether one was live. This joins
 * the bundled catalog against the real connection list (`ListMcpServers`), so a
 * connector reads as **dialed** only when the runtime says so — the health dot is
 * server-derived, never inferred from the connector merely existing.
 *
 * Each connector is a separate process the runtime dials over MCP, never linked
 * into the gateway. Credentials are supplied by REFERENCE: the operator stores a
 * secret by name and the connector resolves it inside its own process, so a
 * credential value never reaches the model, a log, or this page.
 */

import { useMemo } from "react";
import { toUiError } from "../../kx/errors";
import { useListMcpServers } from "../../kx/use-connections";
import { healthDot } from "../../lib/connection-health";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { Badge } from "../ds/Badge";
import { GlowCard } from "../ds/GlowCard";

/** One connector shipped with this runtime. */
interface BundledConnector {
  /** The conventional connection name — also what the dial command uses. */
  name: string;
  provider: string;
  /** The binary the runtime spawns. */
  command: string;
  /** The secret NAME the connector resolves in its own process. */
  credentialRef: string;
  /** The tools it exposes once dialed. */
  tools: readonly string[];
}

/**
 * The bundled set, as shipped. Static because it IS static — these are compiled
 * into the release, not discovered at runtime. A connector the operator built
 * themselves shows up under Connections once dialed, which is the surface for
 * anything not in this list.
 */
const BUNDLED: readonly BundledConnector[] = [
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
];

export function IntegrationsPanel() {
  const { servers, notWired, isLoading, isError, error, refetch } = useListMcpServers();

  /** Health per bundled connector, by connection name. Absent ⇒ never dialed. */
  const dialed = useMemo(() => {
    const byName = new Map<string, string>();
    for (const server of servers) {
      byName.set(server.serverName, server.health);
    }
    return byName;
  }, [servers]);

  if (isLoading) {
    return <EmptyState title="Loading integrations…" />;
  }
  if (isError && !notWired) {
    return <ErrorNotice error={toUiError(error)} onRetry={() => void refetch()} />;
  }

  return (
    <div data-testid="integrations-panel">
      <p className="muted" data-testid="integrations-note">
        Connectors shipped with this runtime. Each is a separate process the runtime dials over MCP
        — never linked into the gateway. Credentials are supplied by <strong>reference</strong>: you
        store a secret by name, and the connector resolves it inside its own process, so the value
        never reaches the model or this page.
        {notWired ? " This gateway does not report connection health (an older build)." : null}
      </p>
      <ul className="registry-list" data-testid="integrations-list">
        {BUNDLED.map((connector) => (
          <ConnectorRow
            key={connector.name}
            connector={connector}
            health={dialed.get(connector.name)}
            healthKnown={!notWired}
          />
        ))}
      </ul>
    </div>
  );
}

function ConnectorRow({
  connector,
  health,
  healthKnown,
}: {
  connector: BundledConnector;
  health: string | undefined;
  healthKnown: boolean;
}) {
  // An undialed connector and an unknown-health gateway are DIFFERENT states, and
  // collapsing them would show "not dialed" for a connector that may well be live.
  const dot = healthDot(health ?? "");
  return (
    <GlowCard className="registry-row" stripe="var(--primary)">
      <div className="registry-row__main">
        <div className="registry-row__head">
          {healthKnown ? (
            <span
              className={`status-dot ${dot.cls}`}
              aria-label={health ? dot.label : "not dialed"}
              data-testid={`integration-health-${connector.name}`}
            />
          ) : null}
          <span className="registry-row__name mono">{connector.name}</span>
          <Badge label={connector.provider} color="var(--primary)" />
          {healthKnown && health ? (
            <Badge
              label={dot.label}
              color={health === "connected" ? "var(--success)" : "var(--warning)"}
            />
          ) : (
            <Badge label={healthKnown ? "not dialed" : "health unknown"} color="var(--text-2)" />
          )}
        </div>
        <p className="registry-row__desc muted">
          Tools: <span className="mono">{connector.tools.join(" · ")}</span>
        </p>
        <dl className="registry-row__meta">
          <div>
            <dt className="muted">command</dt>
            <dd className="mono">{connector.command}</dd>
          </div>
          <div>
            <dt className="muted">credential</dt>
            <dd className="mono">{connector.credentialRef}</dd>
          </div>
        </dl>
        {!health ? (
          <p
            className="registry-row__desc mono"
            data-testid={`integration-dial-${connector.name}`}
            title="Store the credential, then dial the connector"
          >
            {`kx secrets set --name ${connector.credentialRef} --value '…'`}
            <br />
            {`kx connections add --name ${connector.name} --command ${connector.command} --credential-ref ${connector.credentialRef}`}
          </p>
        ) : null}
      </div>
    </GlowCard>
  );
}
