import { MoteTelemetryRow, ReRankTurn, ReplanRound } from "@kortecx/sdk/web";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import React from "react";
import { describe, expect, it, vi } from "vitest";

// ConnectionsHealth (folded into the overview's Gateway-health card) renders a router
// <Link to="/tools">; stub it to a plain <a> so the section renders without a
// RouterProvider (real navigation is covered by the monitoring Playwright spec).
vi.mock("@tanstack/react-router", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@tanstack/react-router")>();
  return {
    ...actual,
    Link: ({ to, params, activeProps, children, ...rest }: any) =>
      React.createElement("a", { href: typeof to === "string" ? to : "#", ...rest }, children),
  };
});

import { MonitoringSection } from "../../src/components/sections/MonitoringSection";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

/** A typed-as-unimplemented throw (errors.ts duck-types `.code` → "not-wired"). */
function unimplemented(): never {
  throw Object.assign(new Error("not wired"), { code: "unimplemented" });
}

describe("MonitoringSection", () => {
  it("renders real aggregated numbers from the gateway RPCs", async () => {
    const mock = makeMockClient({
      listRuns: async () => ({
        runs: [{ instanceId: "i1", recipeFingerprint: "f", registeredUnixMs: 1 }],
        hasMore: false,
      }),
      listReplanRounds: async () => ({
        rounds: [new ReplanRound(0, "aabbcc", "qwen3", ["s1"], false, 9)],
        hasMore: false,
      }),
    });
    render(<MonitoringSection />, { wrapper: connectedWrapper(mock.client) });
    expect(screen.getByTestId("monitoring-section")).toBeInTheDocument();
    // The replan model rolls up into a real tally row + a trail-table row (not a
    // placeholder) — both surface the resolved model id.
    await waitFor(() => expect(screen.getAllByText("qwen3").length).toBeGreaterThan(0));
  });

  it("folds the live re-rank trail into a real ReRank rounds panel", async () => {
    const mock = makeMockClient({
      listRerankTurns: async () => ({
        turns: [new ReRankTurn(0, "aabb", "i1", "gemma3", "reranked", 3, [2, 0, 1], 41)],
        hasMore: false,
      }),
    });
    render(<MonitoringSection />, { wrapper: connectedWrapper(mock.client) });
    // The re-rank turn rolls up into the outcome tally AND a trail-table row (not a
    // placeholder) — both surface the resolved model + settled outcome. ("ReRank
    // rounds" is BOTH the metric-card label and the panel heading, so target the h2.)
    await waitFor(() =>
      expect(screen.getByRole("heading", { name: "ReRank rounds" })).toBeInTheDocument(),
    );
    await waitFor(() => expect(screen.getAllByText("gemma3").length).toBeGreaterThan(0));
    expect(screen.getAllByText("reranked").length).toBeGreaterThan(0);
    // The enforced permutation is rendered honestly (the reordered source indices).
    expect(screen.getByText("2 0 1")).toBeInTheDocument();
  });

  it("exposes the Runs tab (POC-5c: run history moved here from Workflows)", async () => {
    const mock = makeMockClient();
    render(<MonitoringSection />, { wrapper: connectedWrapper(mock.client) });
    // The toggle wires the new Runs view (the RunsTable itself needs a router, so
    // the full run-history render is covered by the shell e2e — here we pin that the
    // tab is present and labelled, never silently dropped).
    expect(screen.getByTestId("monitor-tab-runs")).toHaveTextContent("Runs");
  });

  it("degrades an unimplemented RPC to a muted 'not wired' note", async () => {
    const mock = makeMockClient({ listReplanRounds: async () => unimplemented() });
    render(<MonitoringSection />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getByText(/Not wired on this gateway/i)).toBeInTheDocument());
  });

  it("Telemetry tab shows a per-model rollup + honest-disabled cost tile (no faked cost)", async () => {
    const mock = makeMockClient({
      listMoteTelemetry: async () => ({
        // Unattributed rows ("" instanceId) so the raw table renders "—" rather
        // than a <Link> (the harness has no RouterProvider); the rollup groups by
        // model regardless.
        rows: [
          new MoteTelemetryRow("m1", "", 10, null, 5, "qwen3", "", 0, 2),
          new MoteTelemetryRow("m2", "", 30, null, 7, "qwen3", "", 0, 1),
        ],
        hasMore: false,
      }),
    });
    render(<MonitoringSection tab="telemetry" />, { wrapper: connectedWrapper(mock.client) });
    // The per-model rollup renders the resolved model id, honestly windowed.
    await waitFor(() => expect(screen.getByTestId("telemetry-by-model")).toBeInTheDocument());
    expect(screen.getByText(/not all-time/i)).toBeInTheDocument();
    expect(screen.getAllByText("qwen3").length).toBeGreaterThan(0);
    // Cost is honest-disabled (Cloud), never a fabricated number.
    expect(screen.getByTestId("cost-tile-disabled")).toBeInTheDocument();
  });
});

describe("MonitoringSection — Cost tab (RC6a)", () => {
  const priced = () => ({
    instanceId: "aa".repeat(16),
    turns: 3,
    toolCalls: 2,
    estimatedMicroUsd: 1500,
    ceilingMicroUsd: 0,
    perTurnMicroUsd: 500,
    perToolCallMicroUsd: 0,
    overCeiling: false,
  });

  it("renders turns / tool-calls / estimated spend for a run (GetRunCost)", async () => {
    const mock = makeMockClient({ costGetRunCost: async () => priced() });
    render(<MonitoringSection tab="cost" />, { wrapper: connectedWrapper(mock.client) });
    fireEvent.change(screen.getByTestId("cost-run-input"), {
      target: { value: "aa".repeat(16) },
    });
    fireEvent.click(screen.getByTestId("cost-show-btn"));
    await waitFor(() => expect(screen.getByTestId("cost-result")).toBeInTheDocument());
    expect(screen.getByTestId("cost-kpis")).toHaveTextContent("3");
    expect(screen.getByText(/\$0\.0015/)).toBeInTheDocument();
    // A default-OFF ceiling is honestly labelled, never implied as an enforced limit.
    expect(screen.getByText(/not enforced \(OFF\)/i)).toBeInTheDocument();
    expect(screen.queryByTestId("cost-over-ceiling")).not.toBeInTheDocument();
    expect(mock.costGetRunCost).toHaveBeenCalledWith("aa".repeat(16));
  });

  it("shows NO fabricated $ at zero-baseline pricing and flags over-ceiling (GR15)", async () => {
    const mock = makeMockClient({
      costGetRunCost: async () => ({
        instanceId: "bb".repeat(16),
        turns: 9,
        toolCalls: 4,
        estimatedMicroUsd: 0, // zero-baseline price book — no dollar figure to show
        ceilingMicroUsd: 0,
        perTurnMicroUsd: 0,
        perToolCallMicroUsd: 0,
        overCeiling: true,
      }),
    });
    render(<MonitoringSection tab="cost" />, { wrapper: connectedWrapper(mock.client) });
    fireEvent.change(screen.getByTestId("cost-run-input"), {
      target: { value: "bb".repeat(16) },
    });
    fireEvent.click(screen.getByTestId("cost-show-btn"));
    await waitFor(() => expect(screen.getByTestId("cost-result")).toBeInTheDocument());
    // GR15: never invent a dollar figure when the price book is zero-baseline.
    expect(screen.queryByText(/\$/)).not.toBeInTheDocument();
    expect(screen.getByTestId("cost-zero-baseline")).toBeInTheDocument();
    // Over-ceiling is surfaced honestly even with no priced spend.
    expect(screen.getByTestId("cost-over-ceiling")).toBeInTheDocument();
  });

  it("shows a real $0.0000 (not the zero-baseline tile) for a priced run with no activity", async () => {
    const mock = makeMockClient({
      costGetRunCost: async () => ({
        instanceId: "ee".repeat(16),
        turns: 0,
        toolCalls: 0,
        estimatedMicroUsd: 0, // $0, but because there were no billable turns/tool calls…
        ceilingMicroUsd: 0,
        perTurnMicroUsd: 1000, // …NOT because the price book is unset (rates ARE set).
        perToolCallMicroUsd: 500,
        overCeiling: false,
      }),
    });
    render(<MonitoringSection tab="cost" />, { wrapper: connectedWrapper(mock.client) });
    fireEvent.change(screen.getByTestId("cost-run-input"), {
      target: { value: "ee".repeat(16) },
    });
    fireEvent.click(screen.getByTestId("cost-show-btn"));
    await waitFor(() => expect(screen.getByTestId("cost-result")).toBeInTheDocument());
    // A real price book exists ⇒ an honest $0.0000, NOT the "zero-baseline / set rates"
    // tile (which would misreport the configured rates as unset).
    expect(screen.getByText("$0.0000")).toBeInTheDocument();
    expect(screen.queryByTestId("cost-zero-baseline")).not.toBeInTheDocument();
    // The configured rates are shown honestly.
    expect(screen.getByText("$0.0010")).toBeInTheDocument();
  });

  it("degrades to the honest not-wired note without the cost admin", async () => {
    const mock = makeMockClient({ costGetRunCost: async () => unimplemented() });
    render(<MonitoringSection tab="cost" />, { wrapper: connectedWrapper(mock.client) });
    fireEvent.change(screen.getByTestId("cost-run-input"), {
      target: { value: "cc".repeat(16) },
    });
    fireEvent.click(screen.getByTestId("cost-show-btn"));
    await waitFor(() => expect(screen.getByText(/Not wired on this gateway/i)).toBeInTheDocument());
  });

  it("shows only the run-picker until a run id is entered (no premature RPC)", () => {
    const mock = makeMockClient();
    render(<MonitoringSection tab="cost" />, { wrapper: connectedWrapper(mock.client) });
    expect(screen.getByTestId("cost-run-input")).toBeInTheDocument();
    expect(screen.queryByTestId("cost-result")).not.toBeInTheDocument();
    expect(mock.costGetRunCost).not.toHaveBeenCalled();
  });

  it("exposes the Cost tab in the monitoring fieldset and reports clicks", () => {
    const onTab = vi.fn();
    const mock = makeMockClient();
    render(<MonitoringSection tab="cost" onTab={onTab} />, {
      wrapper: connectedWrapper(mock.client),
    });
    expect(screen.getByTestId("monitor-tab-cost")).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByTestId("monitor-tab-quality"));
    expect(onTab).toHaveBeenCalledWith("quality");
  });
});

describe("MonitoringSection — connector health (RC6a)", () => {
  it("folds MCP connector health into the Gateway-health card (read-only + Test)", async () => {
    const mock = makeMockClient({
      listMcpServers: async () => ({
        servers: [
          {
            connectionId: "ab".repeat(8),
            serverName: "github",
            transport: "http",
            endpoint: "https://mcp.example/rpc",
            health: "connected",
            toolCount: 3,
            credentialRefPresent: true,
          },
        ],
        hasMore: false,
      }),
    });
    render(<MonitoringSection />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() =>
      expect(screen.getByTestId("monitor-connection-github")).toBeInTheDocument(),
    );
    expect(screen.getByText(/3 tool\(s\)/)).toBeInTheDocument();
    // Test re-dials by name; Remove is NOT here (revoke stays in Integrations).
    fireEvent.click(screen.getByTestId("monitor-connection-test-github"));
    await waitFor(() => expect(mock.testMcpServer).toHaveBeenCalledWith("github"));
    expect(screen.queryByTestId("monitor-connection-remove-github")).not.toBeInTheDocument();
    // The manage affordance links to Integrations → Connections (where revoke lives).
    expect(screen.getByTestId("monitor-connections-manage")).toHaveAttribute("href", "/tools");
  });

  it("shows the honest empty state when no connectors are registered", async () => {
    const mock = makeMockClient(); // listMcpServers default → { servers: [] }
    render(<MonitoringSection />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() =>
      expect(screen.getByTestId("monitor-connections-empty")).toBeInTheDocument(),
    );
  });

  it("degrades to the not-wired note on a gateway without the MCP gateway", async () => {
    const mock = makeMockClient({ listMcpServers: async () => unimplemented() });
    render(<MonitoringSection />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() =>
      expect(screen.getByTestId("monitor-connections-not-wired")).toBeInTheDocument(),
    );
  });
});
