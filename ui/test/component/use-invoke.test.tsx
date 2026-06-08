import { KxUnimplemented } from "@kortecx/sdk/web";
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useInvoke } from "../../src/kx/use-invoke";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

const INSTANCE = "ab".repeat(16);

describe("useInvoke", () => {
  it("invokes a recipe and returns the hex instance id", async () => {
    const { client, invoke } = makeMockClient({
      // Mirrors the SDK's `Run` (instanceId is a hex getter).
      invoke: async () => ({ instanceId: INSTANCE }),
    });
    const { result } = renderHook(() => useInvoke(), { wrapper: connectedWrapper(client) });
    let id = "";
    await act(async () => {
      id = await result.current.mutateAsync({ handle: "kx/recipes/echo", args: { topic: "x" } });
    });
    expect(id).toBe(INSTANCE);
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
