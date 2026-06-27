/** MM-3 / D110 secret-store hooks — list NAMES, put (write-only value), delete.
 *  The client is mocked; the VALUE never appears on a read (D81). */

import { KxUnimplemented } from "@kortecx/sdk/web";
import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useDeleteSecret, useListSecretNames, usePutSecret } from "../../src/kx/use-secrets";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

const NAME_ROW = { name: "github_token", createdUnixMs: 1_700_000_000_000, updatedUnixMs: 0 };

describe("useListSecretNames", () => {
  it("lists the stored secret NAMES (never a value)", async () => {
    const { client, secretsList } = makeMockClient({
      secretsList: async () => ({ names: [NAME_ROW], hasMore: false }),
    });
    const { result } = renderHook(() => useListSecretNames(), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.names).toEqual([NAME_ROW]);
    expect(result.current.notWired).toBe(false);
    // Names-only paging — the value is never requested.
    expect(secretsList).toHaveBeenCalledWith({ limit: 200 });
  });

  it("degrades to notWired on an UNIMPLEMENTED gateway", async () => {
    const { client } = makeMockClient({
      secretsList: async () => {
        throw new KxUnimplemented("secret store not wired");
      },
    });
    const { result } = renderHook(() => useListSecretNames(), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.notWired).toBe(true);
  });
});

describe("usePutSecret / useDeleteSecret", () => {
  it("stores a secret (name + write-only value)", async () => {
    const { client, secretsSet } = makeMockClient();
    const { result } = renderHook(() => usePutSecret(), { wrapper: connectedWrapper(client) });
    await act(async () => {
      await result.current.mutateAsync({ name: "github_token", value: "ghp_secret" });
    });
    expect(secretsSet).toHaveBeenCalledWith("github_token", "ghp_secret");
  });

  it("deletes a secret by name", async () => {
    const { client, secretsRemove } = makeMockClient();
    const { result } = renderHook(() => useDeleteSecret(), { wrapper: connectedWrapper(client) });
    await act(async () => {
      await result.current.mutateAsync("github_token");
    });
    expect(secretsRemove).toHaveBeenCalledWith("github_token");
  });
});
