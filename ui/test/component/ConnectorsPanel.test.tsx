/** ConnectorsPanel — the merged catalog-and-instance surface.
 *
 *  Replaces the ConnectionsPanel and IntegrationsPanel tests, which each covered one half
 *  of the same registry. The assertions that matter are about the MERGE:
 *
 *  - a bundled connector appears whether or not it is set up, and its row says which;
 *  - setting one up is an action ON the row, not a command to copy elsewhere;
 *  - "not set up" and "the gateway cannot tell us" stay distinguishable — collapsing them
 *    would state a fact about the server that the runtime never reported;
 *  - a server needing several environment settings can express them, and only NAMES ever
 *    reach the surface.
 *
 *  The kx hooks are mocked, so this is a pure render/interaction check. */

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

import { ConnectorsPanel } from "../../src/components/tools/ConnectorsPanel";

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
  listState.isLoading = false;
  listState.isError = false;
  [registerM, testM, discoverM, removeM, fireM].forEach(resetMut);
});

const configured = (over: Record<string, unknown> = {}) => ({
  connectionId: "aa".repeat(16),
  serverName: "slack",
  transport: "stdio",
  endpoint: "kx-connector-slack",
  health: "connected",
  toolCount: 4,
  credentialRefPresent: true,
  sessionMode: "stateless",
  envNames: [] as string[],
  ...over,
});

describe("ConnectorsPanel", () => {
  it("lists every bundled connector with nothing configured, each offering set-up", () => {
    render(<ConnectorsPanel />);
    for (const name of ["gmail", "slack", "discord", "notion"]) {
      expect(screen.getByTestId(`connector-${name}`)).toBeInTheDocument();
      // The action is ON the row — the old surface could only print a shell command.
      expect(screen.getByTestId(`connector-configure-${name}`)).toBeInTheDocument();
    }
    // All four, since nothing is configured — the count is the assertion.
    expect(screen.getAllByText("not set up", { exact: false })).toHaveLength(4);
  });

  it("shows a configured connector in the SAME list, with live actions instead of set-up", () => {
    listState.servers = [configured()];
    render(<ConnectorsPanel />);
    // One list: the configured connector is the same row, in a different state.
    expect(screen.getByTestId("connector-slack")).toBeInTheDocument();
    expect(screen.queryByTestId("connector-configure-slack")).not.toBeInTheDocument();
    expect(screen.getByTestId("connector-test-slack")).toBeInTheDocument();
    expect(screen.getByTestId("connector-health-slack")).toBeInTheDocument();
    // …while the connectors that are not set up keep their set-up action.
    expect(screen.getByTestId("connector-configure-gmail")).toBeInTheDocument();
  });

  it("keeps a dialed server that is not bundled visible alongside the catalog", () => {
    listState.servers = [configured({ serverName: "gitlab", endpoint: "mcp-server-gitlab" })];
    render(<ConnectorsPanel />);
    expect(screen.getByTestId("connector-gitlab")).toBeInTheDocument();
    expect(screen.getByTestId("connector-gmail")).toBeInTheDocument();
  });

  it("names the environment settings a connector is configured with, and nothing more", () => {
    listState.servers = [
      configured({
        serverName: "gitlab",
        envNames: ["GITLAB_PERSONAL_ACCESS_TOKEN", "GITLAB_API_URL"],
      }),
    ];
    render(<ConnectorsPanel />);
    const names = screen.getByTestId("connector-env-names-gitlab");
    expect(names).toHaveTextContent("GITLAB_PERSONAL_ACCESS_TOKEN");
    expect(names).toHaveTextContent("GITLAB_API_URL");
  });

  it("configures a bundled connector from its own row, prefilled", () => {
    render(<ConnectorsPanel />);
    fireEvent.click(screen.getByTestId("connector-configure-slack"));
    expect(screen.getByTestId("connector-name")).toHaveValue("slack");
    expect(screen.getByTestId("connector-endpoint")).toHaveValue("kx-connector-slack");
    expect(screen.getByTestId("connector-credential")).toHaveValue("KX_SLACK_CREDENTIAL");
  });

  it("submits SEVERAL environment settings, each as a NAME pair", () => {
    render(<ConnectorsPanel />);
    fireEvent.click(screen.getByTestId("connector-add-other"));
    fireEvent.change(screen.getByTestId("connector-name"), { target: { value: "gitlab" } });
    fireEvent.change(screen.getByTestId("connector-endpoint"), {
      target: { value: "mcp-server-gitlab" },
    });

    fireEvent.click(screen.getByTestId("connector-env-add"));
    fireEvent.change(screen.getByTestId("connector-env-name-0"), {
      target: { value: "GITLAB_PERSONAL_ACCESS_TOKEN" },
    });
    fireEvent.change(screen.getByTestId("connector-env-ref-0"), {
      target: { value: "gitlab-token" },
    });
    fireEvent.click(screen.getByTestId("connector-env-add"));
    fireEvent.change(screen.getByTestId("connector-env-name-1"), {
      target: { value: "GITLAB_API_URL" },
    });
    fireEvent.change(screen.getByTestId("connector-env-ref-1"), {
      target: { value: "gitlab-url" },
    });

    fireEvent.click(screen.getByTestId("connector-submit"));

    expect(registerM.mutate).toHaveBeenCalledTimes(1);
    const [input] = registerM.mutate.mock.calls[0] as [Record<string, unknown>];
    // TWO entries — the shape a single credential field cannot express.
    expect(input.env).toEqual({
      GITLAB_PERSONAL_ACCESS_TOKEN: "gitlab-token",
      GITLAB_API_URL: "gitlab-url",
    });
  });

  it("drops a half-typed environment row rather than refusing the whole form", () => {
    render(<ConnectorsPanel />);
    fireEvent.click(screen.getByTestId("connector-add-other"));
    fireEvent.change(screen.getByTestId("connector-name"), { target: { value: "x" } });
    fireEvent.change(screen.getByTestId("connector-endpoint"), { target: { value: "y" } });
    fireEvent.click(screen.getByTestId("connector-env-add"));
    fireEvent.change(screen.getByTestId("connector-env-name-0"), {
      target: { value: "ONLY_NAME" },
    });
    fireEvent.click(screen.getByTestId("connector-submit"));

    const [input] = registerM.mutate.mock.calls[0] as [Record<string, unknown>];
    expect(input.env).toEqual({});
  });

  it("removes an environment row", () => {
    render(<ConnectorsPanel />);
    fireEvent.click(screen.getByTestId("connector-add-other"));
    fireEvent.click(screen.getByTestId("connector-env-add"));
    expect(screen.getByTestId("connector-env-0")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("connector-env-remove-0"));
    expect(screen.queryByTestId("connector-env-0")).not.toBeInTheDocument();
  });

  it("offers no environment settings for a remote URL — they configure a child process", () => {
    render(<ConnectorsPanel />);
    fireEvent.click(screen.getByTestId("connector-add-other"));
    expect(screen.getByTestId("connector-env")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("connector-transport-http"));
    expect(screen.queryByTestId("connector-env")).not.toBeInTheDocument();
  });

  it("says the gateway cannot dial at all, rather than calling everything not set up", () => {
    listState.notWired = true;
    render(<ConnectorsPanel />);
    // The distinction the old panel was careful about survives the merge: an absent
    // capability is reported as such, never as a claim about any connector's state.
    expect(screen.getByText(/not available on this gateway/i)).toBeInTheDocument();
    expect(screen.queryByTestId("connector-gmail")).not.toBeInTheDocument();
  });

  it("surfaces a per-row action refusal instead of swallowing it", () => {
    listState.servers = [configured()];
    testM.error = { code: "permission-denied", message: "nope" };
    render(<ConnectorsPanel />);
    expect(screen.getByTestId("connector-action-error")).toBeInTheDocument();
  });

  it("fires a tool from a configured row", () => {
    listState.servers = [configured()];
    render(<ConnectorsPanel />);
    fireEvent.click(screen.getByTestId("connector-fire-toggle-slack"));
    fireEvent.change(screen.getByTestId("connector-fire-tool-slack"), {
      target: { value: "search" },
    });
    fireEvent.click(screen.getByTestId("connector-fire-run-slack"));
    expect(fireM.mutate).toHaveBeenCalledWith({ name: "slack", tool: "search", args: "{}" });
  });
});
