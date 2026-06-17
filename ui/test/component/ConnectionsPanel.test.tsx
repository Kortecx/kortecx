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
const registerMutate = vi.fn();
const testMutate = vi.fn();
const discoverMutate = vi.fn();
const removeMutate = vi.fn();

const idleMutation = (mutate: ReturnType<typeof vi.fn>) => ({
  mutate,
  isPending: false,
  variables: undefined,
  error: null,
  isSuccess: false,
  data: undefined,
});

vi.mock("../../src/kx/use-connections", () => ({
  useListMcpServers: () => listState,
  useRegisterMcpServer: () => idleMutation(registerMutate),
  useTestMcpServer: () => idleMutation(testMutate),
  useDiscoverServerTools: () => idleMutation(discoverMutate),
  useDeregisterMcpServer: () => idleMutation(removeMutate),
}));

import { ConnectionsPanel } from "../../src/components/tools/ConnectionsPanel";

afterEach(() => {
  listState.servers = [];
  listState.notWired = false;
  registerMutate.mockClear();
  testMutate.mockClear();
  discoverMutate.mockClear();
  removeMutate.mockClear();
});

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
    expect(testMutate).toHaveBeenCalledWith("github");
    fireEvent.click(screen.getByTestId("connection-remove-github"));
    expect(removeMutate).toHaveBeenCalledWith("github");
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
    expect(registerMutate).toHaveBeenCalledTimes(1);
    const [input] = registerMutate.mock.calls[0];
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
});
