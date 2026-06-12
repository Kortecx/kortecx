/**
 * Batch C — the Monitoring tabs, telemetry table states, and the global feed.
 * Every D142.3 state is pinned: rows / empty / not-wired degrade for telemetry;
 * empty + rows + run-attribution for the global feed; the tab fieldset's
 * aria-pressed + onTab contract.
 */

import { GlobalDelta, MoteTelemetryRow } from "@kortecx/sdk/web";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { GlobalFeed } from "../../src/components/activity/GlobalFeed";
import { MonitoringSection } from "../../src/components/sections/MonitoringSection";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

function unimplemented(): never {
  throw Object.assign(new Error("not wired"), { code: "unimplemented" });
}

/** Rows are UNATTRIBUTED ("" instance → the "—" cell) here: the attributed
 *  variant renders a router Link, which the monitoring-feed Playwright spec
 *  covers against the real router. */
function telemetryRow(seq: number): MoteTelemetryRow {
  return new MoteTelemetryRow(
    "ab".repeat(32),
    "",
    7,
    null,
    42,
    "kx-serve:qwen3",
    "mcp-echo@1",
    1_700_000_000_000,
    seq,
  );
}

describe("MonitoringSection (Batch C tabs)", () => {
  it("the telemetry tab renders joined rows + load-more on hasMore", async () => {
    const mock = makeMockClient({
      listMoteTelemetry: async () => ({ rows: [telemetryRow(9), telemetryRow(8)], hasMore: true }),
    });
    render(<MonitoringSection tab="telemetry" />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getAllByTestId("telemetry-row")).toHaveLength(2));
    // Real exhaust fields, honestly rendered; the unattributed run cell is "—".
    expect(screen.getAllByText("kx-serve:qwen3").length).toBeGreaterThan(0);
    expect(screen.getAllByText("mcp-echo@1").length).toBeGreaterThan(0);
    expect(screen.getAllByText("—").length).toBeGreaterThan(0);
    expect(screen.getByTestId("telemetry-load-more")).toBeInTheDocument();
  });

  it("telemetry degrades to the honest not-wired note on an old gateway", async () => {
    const mock = makeMockClient({ listMoteTelemetry: async () => unimplemented() });
    render(<MonitoringSection tab="telemetry" />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() =>
      expect(screen.getByText(/Not wired on this gateway/i)).toBeInTheDocument(),
    );
  });

  it("telemetry shows the actionable empty state when no mote has executed", async () => {
    const mock = makeMockClient();
    render(<MonitoringSection tab="telemetry" />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getByText(/No telemetry yet/i)).toBeInTheDocument());
  });

  it("the tab fieldset reflects the active tab and reports clicks via onTab", async () => {
    const onTab = vi.fn();
    const mock = makeMockClient();
    render(<MonitoringSection tab="feed" onTab={onTab} />, {
      wrapper: connectedWrapper(mock.client),
    });
    expect(screen.getByTestId("monitor-tab-feed")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("monitor-tab-overview")).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(screen.getByTestId("monitor-tab-telemetry"));
    expect(onTab).toHaveBeenCalledWith("telemetry");
    fireEvent.click(screen.getByTestId("monitor-tab-overview"));
    expect(onTab).toHaveBeenCalledWith(undefined);
  });
});

describe("GlobalFeed (Batch C)", () => {
  it("renders attributed rows — run starts labelled, commits linked to their run", async () => {
    const deltas = [
      new GlobalDelta(1, "run_registered", "aa".repeat(16), null, null, null, null, null, null,
        "bb".repeat(32), 123),
      new GlobalDelta(2, "committed", "aa".repeat(16), "cc".repeat(32), "dd".repeat(32), 0),
    ];
    const mock = makeMockClient({
      // Yield both rows then stay open (the live-tail shape).
      wsAllEvents: async function* () {
        yield* deltas;
        await new Promise(() => {});
      },
    });
    // The handler variant renders run chips as buttons (the Link variant needs
    // a real router — covered by the monitoring-feed Playwright spec).
    render(<GlobalFeed onSelectRun={() => {}} />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getAllByTestId("global-event-row")).toHaveLength(2));
    expect(screen.getByText("RUN STARTED")).toBeInTheDocument();
    expect(screen.getByText(/Run started/)).toBeInTheDocument();
    // Both rows carry the run chip (the click-through affordance).
    expect(screen.getAllByTestId("global-event-run")).toHaveLength(2);
  });

  it("drives onSelectRun from a row's run chip (the drawer drill-in)", async () => {
    const onSelectRun = vi.fn();
    const mock = makeMockClient({
      wsAllEvents: async function* () {
        yield new GlobalDelta(2, "committed", "aa".repeat(16), "cc".repeat(32), "dd".repeat(32), 0);
        await new Promise(() => {});
      },
    });
    render(<GlobalFeed onSelectRun={onSelectRun} />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getByTestId("global-event-run")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("global-event-run"));
    expect(onSelectRun).toHaveBeenCalledWith("aa".repeat(16));
  });

  it("shows the listening empty state while the live stream is quiet", async () => {
    const mock = makeMockClient({
      wsAllEvents: async function* () {
        await new Promise(() => {}); // open, never yields
      },
    });
    render(<GlobalFeed />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getByText(/Listening for events/i)).toBeInTheDocument());
  });

  it("degrades honestly when the stream fails before any frame (old gateway)", async () => {
    const mock = makeMockClient({
      // The old-bridge shape: the handshake rejects before any frame.
      wsAllEvents: async function* () {
        throw new Error("Unexpected server response: 400");
      },
    });
    render(<GlobalFeed />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getByText(/Live feed unavailable/i)).toBeInTheDocument());
    expect(screen.getByTestId("global-feed-retry")).toBeInTheDocument();
  });
});
