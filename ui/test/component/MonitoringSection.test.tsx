import { MoteTelemetryRow, ReplanRound } from "@kortecx/sdk/web";
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
