import { KxUnauthenticated } from "@kortecx/sdk/web";
import { QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  type ClientFactory,
  KxConnectionProvider,
  useConnection,
} from "../../src/kx/connection-context";
import { makeTestQueryClient } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

const EP = "http://127.0.0.1:50151";

function realWrapper(factory: ClientFactory) {
  const qc = makeTestQueryClient();
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <KxConnectionProvider createClient={factory}>{children}</KxConnectionProvider>
      </QueryClientProvider>
    );
  };
}

afterEach(() => {
  localStorage.clear();
  vi.restoreAllMocks();
});

describe("connection machine", () => {
  it("connects after a successful probe", async () => {
    const { client } = makeMockClient({ listSignatures: async () => [] });
    const factory = vi.fn(() => client);
    const { result } = renderHook(() => useConnection(), { wrapper: realWrapper(factory) });

    await act(async () => {
      const ok = await result.current.connect(EP);
      expect(ok).toBe(true);
    });
    expect(result.current.status).toBe("connected");
    expect(result.current.client).toBe(client);
    expect(result.current.error).toBeNull();
  });

  it("surfaces an auth failure and stays disconnected", async () => {
    const { client, close } = makeMockClient({
      listSignatures: async () => {
        throw new KxUnauthenticated("no token");
      },
    });
    const { result } = renderHook(() => useConnection(), { wrapper: realWrapper(() => client) });

    await act(async () => {
      const ok = await result.current.connect(EP, "bad-token");
      expect(ok).toBe(false);
    });
    expect(result.current.status).toBe("disconnected");
    expect(result.current.error?.kind).toBe("reauth");
    expect(close).toHaveBeenCalled();
  });

  it("NEVER persists the bearer token (only the endpoint)", async () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem");
    const { client } = makeMockClient({ listSignatures: async () => [] });
    const { result } = renderHook(() => useConnection(), { wrapper: realWrapper(() => client) });

    const TOKEN = "super-secret-token-value";
    await act(async () => {
      await result.current.connect(EP, TOKEN);
    });

    // The endpoint is persisted; the token never is.
    const persistedValues = setItem.mock.calls.map((c) => String(c[1]));
    expect(persistedValues).toContain(EP);
    expect(persistedValues.some((v) => v.includes(TOKEN))).toBe(false);
    expect(JSON.stringify(localStorage)).not.toContain(TOKEN);
  });

  it("disconnect tears down the client and clears state", async () => {
    const { client, close } = makeMockClient({ listSignatures: async () => [] });
    const { result } = renderHook(() => useConnection(), { wrapper: realWrapper(() => client) });
    await act(async () => {
      await result.current.connect(EP);
    });
    act(() => {
      result.current.disconnect();
    });
    expect(result.current.status).toBe("disconnected");
    expect(result.current.client).toBeNull();
    expect(close).toHaveBeenCalled();
  });
});
