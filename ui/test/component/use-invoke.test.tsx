import { KxUnimplemented } from "@kortecx/sdk/web";
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useInvoke } from "../../src/kx/use-invoke";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

const INSTANCE = "ab".repeat(16);
const TERMINAL = "cd".repeat(32);

describe("useInvoke", () => {
  it("invokes a recipe and returns the run's instance + terminal Mote ids", async () => {
    const { client, invoke } = makeMockClient({
      // Mirrors the SDK's `Run` (instanceId/terminalMoteId are hex getters).
      invoke: async () => ({ instanceId: INSTANCE, terminalMoteId: TERMINAL }),
    });
    const { result } = renderHook(() => useInvoke(), { wrapper: connectedWrapper(client) });
    let run = { instanceId: "", terminalMoteId: "" };
    await act(async () => {
      run = await result.current.mutateAsync({ handle: "kx/recipes/echo", args: { topic: "x" } });
    });
    expect(run).toEqual({ instanceId: INSTANCE, terminalMoteId: TERMINAL });
    expect(invoke).toHaveBeenCalledWith("kx/recipes/echo", { topic: "x" });
  });

  it("propagates an UNIMPLEMENTED invoke (no catalog) for the UI to surface", async () => {
    const { client } = makeMockClient({
      invoke: async () => {
        throw new KxUnimplemented("invoke not wired");
      },
    });
    const { result } = renderHook(() => useInvoke(), { wrapper: connectedWrapper(client) });
    await act(async () => {
      await expect(
        result.current.mutateAsync({ handle: "x/y/z", args: {} }),
      ).rejects.toBeInstanceOf(KxUnimplemented);
    });
  });
});
