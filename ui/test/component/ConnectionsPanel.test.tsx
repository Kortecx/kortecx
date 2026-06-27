/** PR-6b-1 ConnectionsPanel — the live external-MCP-gateway govern surface:
 *  the not-wired / empty / list states, the per-row actions, and the add form
 *  (transport chips, server fields). The kx hooks are mocked so the test is a
 *  pure render/interaction check. */

import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const listState = {
  servers: [] as Array<Record<string, unknown>>,
  notWired: false,
  isLoading: false,
  isError: false,
  error: null as unknown,
  refetch: vi.fn(),
};
// Mutable per-mutation state so tests can flip isSuccess/data/error.
const mut = (mutate: ReturnType<typeof vi.fn>) => ({
  mutate,
  isPending: false,
  variables: undefined as unknown,
  error: null as unknown,
  isSuccess: false,
  data: undefined as unknown,
});
const registerM = mut(vi.fn());
const testM = mut(vi.fn());
const discoverM = mut(vi.fn());
const removeM = mut(vi.fn());
const fireM = mut(vi.fn());

vi.mock("../../src/kx/use-connections", () => ({
  useListMcpServers: () => listState,
  useRegisterMcpServer: () => registerM,
  useTestMcpServer: () => testM,
  useDiscoverServerTools: () => discoverM,
  useDeregisterMcpServer: () => removeM,
  useCallMcpTool: () => fireM,
}));

import { ConnectionsPanel } from "../../src/components/tools/ConnectionsPanel";

function resetMut(m: ReturnType<typeof mut>) {
  m.isPending = false;
  m.variables = undefined;
  m.error = null;
  m.isSuccess = false;
  m.data = undefined;
  m.mutate.mockClear();
}

afterEach(() => {
  listState.servers = [];
  listState.notWired = false;
  [registerM, testM, discoverM, removeM, fireM].forEach(resetMut);
});

const oneServer = () => {
  listState.servers = [
    {
      connectionId: "cd".repeat(8),
      serverName: "refconn",
      transport: "stdio",
      endpoint: "kx-connector-example",
      health: "connected",
      toolCount: 2,
      credentialRefPresent: false,
    },
  ];
};

describe("ConnectionsPanel", () => {
  it("shows the honest not-wired empty state", () => {
    listState.notWired = true;
    render(<ConnectionsPanel />);
    expect(screen.getByText("MCP gateway not enabled")).toBeInTheDocument();
  });

  it("shows the empty state when no servers are connected", () => {
    render(<ConnectionsPanel />);
    expect(screen.getByText("No MCP servers connected")).toBeInTheDocument();
    // The add form + the honest-disabled Cloud affordance are always present.
    expect(screen.getByTestId("connections-add-form")).toBeInTheDocument();
    expect(screen.getByTestId("connections-cloud-disabled")).toBeInTheDocument();
  });

  it("renders a registered server with its health + per-row actions", () => {
    listState.servers = [
      {
        connectionId: "ab".repeat(8),
        serverName: "github",
        transport: "http",
        endpoint: "https://mcp.github.example/rpc",
        health: "connected",
        toolCount: 3,
        credentialRefPresent: true,
      },
    ];
    render(<ConnectionsPanel />);
    expect(screen.getByTestId("connection-github")).toBeInTheDocument();
    expect(screen.getByText("github")).toBeInTheDocument();
    expect(screen.getByText("https://mcp.github.example/rpc")).toBeInTheDocument();
    // Per-row actions fire the right mutations with the server name.
    fireEvent.click(screen.getByTestId("connection-test-github"));
    expect(testM.mutate).toHaveBeenCalledWith("github");
    fireEvent.click(screen.getByTestId("connection-remove-github"));
    expect(removeM.mutate).toHaveBeenCalledWith("github");
  });

  it("surfaces a per-action result (test reachable) and error (remove failed)", () => {
    // review #3: the per-row mutations must not be silent — D142 every state.
    testM.isSuccess = true;
    testM.data = true;
    const { rerender } = render(<ConnectionsPanel />);
    expect(screen.getByTestId("connection-action-result")).toHaveTextContent("reachable");

    testM.isSuccess = false;
    testM.data = undefined;
    removeM.error = { code: 5, message: "no such MCP server: gone" };
    rerender(<ConnectionsPanel />);
    expect(screen.getByTestId("connection-action-error")).toBeInTheDocument();
  });

  it("submits the add form with the chosen transport + fields", () => {
    render(<ConnectionsPanel />);
    // Default transport is stdio → the args field is shown (not the TLS toggle).
    fireEvent.change(screen.getByTestId("connection-name"), { target: { value: "local" } });
    fireEvent.change(screen.getByTestId("connection-endpoint"), {
      target: { value: "my-server" },
    });
    fireEvent.change(screen.getByTestId("connection-args"), { target: { value: "--stdio -v" } });
    fireEvent.submit(screen.getByTestId("connections-add-form"));
    expect(registerM.mutate).toHaveBeenCalledTimes(1);
    const input = registerM.mutate.mock.calls[0]?.[0];
    expect(input).toMatchObject({
      name: "local",
      transport: "stdio",
      endpoint: "my-server",
      args: ["--stdio", "-v"],
    });
  });

  it("switches to http transport and shows the TLS toggle", () => {
    render(<ConnectionsPanel />);
    fireEvent.click(screen.getByTestId("connection-transport-http"));
    expect(screen.getByTestId("connection-tls")).toBeInTheDocument();
  });

  it("fires a tool from the per-row live-fire panel and shows the result", () => {
    oneServer();
    fireM.isSuccess = true;
    fireM.data = { ok: true, resultJson: '{"reversed":"gnop"}', error: "" };
    render(<ConnectionsPanel />);
    // The fire panel is collapsed until toggled (no clutter on the row).
    expect(screen.queryByTestId("connection-fire-form-refconn")).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId("connection-fire-toggle-refconn"));
    fireEvent.change(screen.getByTestId("connection-fire-tool-refconn"), {
      target: { value: "reverse" },
    });
    fireEvent.change(screen.getByTestId("connection-fire-args-refconn"), {
      target: { value: '{"text":"pong"}' },
    });
    fireEvent.click(screen.getByTestId("connection-fire-run-refconn"));
    expect(fireM.mutate).toHaveBeenCalledWith({
      name: "refconn",
      tool: "reverse",
      args: '{"text":"pong"}',
    });
    expect(screen.getByTestId("connection-fire-result-refconn")).toHaveTextContent("gnop");
  });

  it("surfaces a live-fire failure ({ ok:false, error }) inline, never silent", () => {
    oneServer();
    fireM.isSuccess = true;
    fireM.data = { ok: false, resultJson: "", error: "no registered tool `refconn/nope`" };
    render(<ConnectionsPanel />);
    fireEvent.click(screen.getByTestId("connection-fire-toggle-refconn"));
    fireEvent.change(screen.getByTestId("connection-fire-tool-refconn"), {
      target: { value: "nope" },
    });
    fireEvent.click(screen.getByTestId("connection-fire-run-refconn"));
    expect(screen.getByTestId("connection-fire-error-refconn")).toHaveTextContent(
      "no registered tool",
    );
  });
});
