/**
 * A3: the composer MCP connection chips — honest connection status (connected /
 * unreachable / not dialed) reusing the shared health-dot mapping. A chip is a
 * CONNECTION status, never a fire guarantee. Renders nothing with no servers.
 */

import type { McpServer } from "@kortecx/sdk/web";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { McpConnectionChips } from "../../src/components/chat/McpConnectionChips";

function server(over: Partial<McpServer>): McpServer {
  return {
    connectionId: "c1",
    serverName: "mcp-echo",
    transport: "stdio",
    endpoint: "stdio://echo",
    health: "connected",
    toolCount: 1,
    credentialRefPresent: false,
    sessionMode: "stateless",
    ...over,
  } as McpServer;
}

describe("McpConnectionChips (A3)", () => {
  it("renders one chip per server with the health dot + tool count", () => {
    render(
      <McpConnectionChips
        servers={[
          server({ connectionId: "c1", serverName: "mcp-echo", health: "connected", toolCount: 2 }),
          server({ connectionId: "c2", serverName: "docs", health: "unreachable", toolCount: 0 }),
        ]}
      />,
    );
    expect(screen.getByTestId("chat-mcp-strip")).toBeInTheDocument();
    const echo = screen.getByTestId("chat-mcp-chip-mcp-echo");
    expect(echo.textContent).toContain("mcp-echo");
    expect(echo.textContent).toContain("2 tool(s)");
    // Health maps to the shared status-dot palette + a11y label.
    expect(echo.querySelector(".status-dot--online")?.getAttribute("aria-label")).toBe("connected");
    const docs = screen.getByTestId("chat-mcp-chip-docs");
    expect(docs.querySelector(".status-dot--error")?.getAttribute("aria-label")).toBe(
      "unreachable",
    );
  });

  it("renders nothing when no servers are connected (never a fake row)", () => {
    const { container } = render(<McpConnectionChips servers={[]} />);
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByTestId("chat-mcp-strip")).toBeNull();
  });
});
