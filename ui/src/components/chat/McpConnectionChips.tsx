import type { McpServer } from "@kortecx/sdk/web";
import { healthDot } from "../../lib/connection-health";

/**
 * Honest MCP connection-status chips shown above the composer. A chip reflects the
 * server's dial health (connected / unreachable / not dialed) — it is a CONNECTION
 * status, NOT a guarantee that a tool will fire (authority is re-checked per turn).
 * Renders nothing when no servers are connected (never a fake row).
 */
export function McpConnectionChips({ servers }: { servers: readonly McpServer[] }) {
  if (servers.length === 0) {
    return null;
  }
  return (
    <div className="context-strip" data-testid="chat-mcp-strip">
      <span className="context-strip__label muted">Connections:</span>
      {servers.map((s) => {
        const dot = healthDot(s.health);
        return (
          <span
            key={s.connectionId}
            className="context-strip__chip"
            data-testid={`chat-mcp-chip-${s.serverName}`}
            title={`${s.transport} · ${s.toolCount} tool(s) · ${dot.label}`}
          >
            <span className={`status-dot ${dot.cls}`} role="img" aria-label={dot.label} />
            <span className="mono">{s.serverName}</span>
            <span className="muted">· {s.toolCount} tool(s)</span>
          </span>
        );
      })}
    </div>
  );
}
