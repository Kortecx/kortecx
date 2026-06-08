import { ErrorCode } from "@kortecx/sdk/web";
import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useHealth } from "../../src/kx/use-health";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

describe("useHealth", () => {
  it("a successful probe → live", async () => {
    const { client } = makeMockClient({ listSignatures: async () => [] });
    const { result } = renderHook(() => useHealth(), { wrapper: connectedWrapper(client) });
    await waitFor(() => expect(result.current.data).toBe("live"));
  });

  it("reachable but UNIMPLEMENTED → still live", async () => {
    const { client } = makeMockClient({
      listSignatures: async () => {
        throw Object.assign(new Error("nope"), { code: ErrorCode.Unimplemented });
      },
    });
    const { result } = renderHook(() => useHealth(), { wrapper: connectedWrapper(client) });
    await waitFor(() => expect(result.current.data).toBe("live"));
  });

  it("transport unreachable → down", async () => {
    const { client } = makeMockClient({
      listSignatures: async () => {
        throw Object.assign(new Error("unreachable"), { code: ErrorCode.Unavailable });
      },
    });
    const { result } = renderHook(() => useHealth(), { wrapper: connectedWrapper(client) });
    await waitFor(() => expect(result.current.data).toBe("down"));
  });
});
