/** IntegrationsPanel — the bundled connectors joined against the live connection
 *  list. The kx hook is mocked → a pure render check.
 *
 *  The assertion that matters is a three-state one. "Dialed", "not dialed" and
 *  "the gateway cannot tell us" are genuinely different, and collapsing the last
 *  two would show "not dialed" for a connector that may well be running — a
 *  fabricated fact about the server, which is the one thing these panels must
 *  never produce. */

import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const state = {
  servers: [] as Array<Record<string, unknown>>,
  notWired: false,
  isLoading: false,
  isError: false,
  error: null as unknown,
  refetch: vi.fn(),
};

vi.mock("../../src/kx/use-connections", () => ({
  useListMcpServers: () => state,
}));

import { IntegrationsPanel } from "../../src/components/tools/IntegrationsPanel";

afterEach(() => {
  state.servers = [];
  state.notWired = false;
  state.isLoading = false;
  state.isError = false;
  state.error = null;
});

describe("IntegrationsPanel", () => {
  it("lists every bundled connector even with nothing dialed", () => {
    render(<IntegrationsPanel />);
    for (const name of ["gmail", "slack", "discord", "notion"]) {
      expect(screen.getByTestId(`integration-health-${name}`)).toBeInTheDocument();
    }
    // Undialed rows carry the commands to store the credential and dial.
    expect(screen.getByTestId("integration-dial-gmail")).toHaveTextContent(
      /kx secrets set --name KX_GMAIL_CREDENTIAL/,
    );
    expect(screen.getByTestId("integration-dial-gmail")).toHaveTextContent(
      /kx connections add --name gmail --command kx-connector-gmail/,
    );
  });

  it("reads a connector as dialed only when the runtime says so", () => {
    state.servers = [{ serverName: "slack", health: "connected" }];
    render(<IntegrationsPanel />);

    expect(screen.getByText("connected")).toBeInTheDocument();
    // A dialed connector no longer shows dial instructions…
    expect(screen.queryByTestId("integration-dial-slack")).toBeNull();
    // …while the others still do.
    expect(screen.getByTestId("integration-dial-gmail")).toBeInTheDocument();
  });

  it("distinguishes an unreachable connector from an undialed one", () => {
    state.servers = [{ serverName: "notion", health: "unreachable" }];
    render(<IntegrationsPanel />);
    expect(screen.getByText("unreachable")).toBeInTheDocument();
    expect(screen.getAllByText("not dialed").length).toBe(3);
  });

  it("says health is UNKNOWN rather than 'not dialed' on an older gateway", () => {
    state.notWired = true;
    render(<IntegrationsPanel />);
    // The distinction this test exists for: a gateway that cannot report health
    // must not be rendered as four connectors that are definitely not running.
    expect(screen.getAllByText("health unknown").length).toBe(4);
    expect(screen.queryByText("not dialed")).toBeNull();
    expect(screen.getByTestId("integrations-note")).toHaveTextContent(
      /does not report connection health/,
    );
  });

  it("surfaces a real load error with a retry", () => {
    state.isError = true;
    state.error = new Error("boom");
    render(<IntegrationsPanel />);
    expect(screen.queryByTestId("integrations-list")).toBeNull();
  });

  it("shows a loading state", () => {
    state.isLoading = true;
    render(<IntegrationsPanel />);
    expect(screen.getByText(/Loading integrations/)).toBeInTheDocument();
  });
});
