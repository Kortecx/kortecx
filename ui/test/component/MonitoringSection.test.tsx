import { MoteTelemetryRow, ReRankTurn, ReplanRound } from "@kortecx/sdk/web";
import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
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
