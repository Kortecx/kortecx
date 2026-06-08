import { Delta } from "@kortecx/sdk/web";
import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useEventStream } from "../../src/kx/use-event-stream";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

const INSTANCE = "ab".repeat(16);

function delta(seq: number, kind = "committed"): Delta {
  return new Delta(seq, kind, "aa".repeat(32), "bb".repeat(32));
}

describe("useEventStream", () => {
  it("accumulates deltas newest-first and goes inactive when the tail ends", async () => {
    const { client } = makeMockClient({
      wsEvents: async function* () {
        yield delta(1);
        yield delta(2, "failed");
      },
    });
    const { result } = renderHook(() => useEventStream(INSTANCE), {
      wrapper: connectedWrapper(client),
    });

    await waitFor(() => expect(result.current.events).toHaveLength(2));
    expect(result.current.events[0]?.seq).toBe(2); // newest-first
    expect(result.current.events[1]?.seq).toBe(1);
    await waitFor(() => expect(result.current.active).toBe(false));
    expect(result.current.dropped).toBe(false);
  });

  it("bounds the buffer to `max` (drops oldest)", async () => {
    const { client } = makeMockClient({
      wsEvents: async function* () {
        yield delta(1);
        yield delta(2);
        yield delta(3);
      },
    });
    const { result } = renderHook(() => useEventStream(INSTANCE, { max: 2 }), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.events).toHaveLength(2));
    expect(result.current.events.map((d) => d.seq)).toEqual([3, 2]);
  });

  it("flags a dropped stream when the tail errors", async () => {
    const { client } = makeMockClient({
      wsEvents: async function* () {
        yield delta(1);
        throw new Error("socket error");
      },
    });
    const { result } = renderHook(() => useEventStream(INSTANCE), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.dropped).toBe(true));
  });

  it("unmounts cleanly (no throw) — the iterator is returned", () => {
    const { client } = makeMockClient({
      wsEvents: async function* () {
        yield delta(1);
      },
    });
    const { unmount } = renderHook(() => useEventStream(INSTANCE), {
      wrapper: connectedWrapper(client),
    });
    expect(() => unmount()).not.toThrow();
  });
});
