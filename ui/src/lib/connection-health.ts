/**
 * Pure mapping of a folded MCP-connection health tag to an existing status-dot
 * modifier + an a11y label. Extracted so the Integrations Connections panel and the
 * Monitoring overview render the SAME dot for the SAME health (one source of truth,
 * no divergent color/label). Health is server-derived (SN-8); the classes are the
 * existing token-driven `.status-dot--*` palette (no hardcoded colour).
 */

/** Map a folded health tag (`connected` / `unreachable` / anything else) to a
 *  `.status-dot--*` modifier class + a human a11y label. */
export function healthDot(health: string): { cls: string; label: string } {
  switch (health) {
    case "connected":
      return { cls: "status-dot--online", label: "connected" };
    case "unreachable":
      return { cls: "status-dot--error", label: "unreachable" };
    default:
      return { cls: "status-dot--offline", label: "not dialed" };
  }
}
