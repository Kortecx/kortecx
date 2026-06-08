import { KxUnimplemented } from "@kortecx/sdk/web";
import { renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { useRuns } from "../../src/kx/use-runs";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

afterEach(() => localStorage.clear());

const BB = "bb".repeat(8);

describe("useRuns (UI-2 ListRuns)", () => {
  it("merges the durable ListRuns enumeration into the run list", async () => {
    const { client } = makeMockClient({
      listRuns: async () => ({
        runs: [{ instanceId: BB, recipeFingerprint: "ff".repeat(32), registeredUnixMs: 99 }],
        hasMore: false,
      }),
    });
    const { result } = renderHook(() => useRuns(), { wrapper: connectedWrapper(client) });
    await waitFor(() => expect(result.current.serverAvailable).toBe(true));
    expect(result.current.notWired).toBe(false);
    expect(result.current.runs.map((r) => r.instanceId)).toContain(BB);
  });

  it("degrades to the local history when ListRuns is UNIMPLEMENTED", async () => {
    const { client } = makeMockClient({
      listRuns: async () => {
        throw new KxUnimplemented("ListRuns not wired");
      },
    });
    const { result } = renderHook(() => useRuns(), { wrapper: connectedWrapper(client) });
    await waitFor(() => expect(result.current.notWired).toBe(true));
    expect(result.current.serverAvailable).toBe(false);
    // No local history + no server runs ⇒ empty (no crash).
    expect(result.current.runs).toEqual([]);
  });
});
