/**
 * The global live-event feed (Batch C) — used by the navbar Activity drawer. Pins
 * every D142.3 state: attributed rows (run starts labelled, commits linked to their
 * run), the run-chip drill-in, the quiet "listening" empty state, and the honest
 * degrade when the live stream fails before any frame.
 */

import { GlobalDelta } from "@kortecx/sdk/web";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { GlobalFeed } from "../../src/components/activity/GlobalFeed";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

describe("GlobalFeed (Batch C)", () => {
  it("renders attributed rows — run starts labelled, commits linked to their run", async () => {
    const deltas = [
      new GlobalDelta(
        1,
        "run_registered",
        "aa".repeat(16),
        null,
        null,
        null,
        null,
        null,
        null,
        "bb".repeat(32),
        123,
      ),
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
    // a real router — covered by the activity-global Playwright spec).
    render(<GlobalFeed onSelectRun={() => {}} />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getAllByTestId("global-event-row")).toHaveLength(2));
    // Scope to the row PILL — the W1a-3 triage toolbar also renders a "RUN STARTED"
    // filter chip, so the bare text now appears twice (chip + pill).
    expect(screen.getByText("RUN STARTED", { selector: ".pill" })).toBeInTheDocument();
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
      // biome-ignore lint/correctness/useYield: a generator that fails BEFORE its first frame is exactly the case under test.
      wsAllEvents: async function* () {
        throw new Error("Unexpected server response: 400");
      },
    });
    render(<GlobalFeed />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getByText(/Live feed unavailable/i)).toBeInTheDocument());
    expect(screen.getByTestId("global-feed-retry")).toBeInTheDocument();
  });
});
