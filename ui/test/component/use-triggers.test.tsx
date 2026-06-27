/** D113 / D170.b trigger-registry hooks — list, register, test (dry-run), fire
 *  (the inbound event), deregister. The client is mocked. */

import { KxUnimplemented } from "@kortecx/sdk/web";
import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  useDeregisterTrigger,
  useFireTrigger,
  useListTriggers,
  useRegisterTrigger,
  useTestTrigger,
} from "../../src/kx/use-triggers";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

const TRIGGER = {
  triggerId: "ab".repeat(8),
  name: "gh-push",
  kind: "webhook",
  recipeHandle: "kx/recipes/react",
  auth: "hmac_sha256",
  authSecretPresent: true,
  scheduleSpec: "",
  enabled: true,
  lastFireUnixMs: 0,
};

describe("useListTriggers", () => {
  it("lists the registered triggers", async () => {
    const { client, triggersList } = makeMockClient({
      triggersList: async () => ({ triggers: [TRIGGER], hasMore: false }),
    });
    const { result } = renderHook(() => useListTriggers(), { wrapper: connectedWrapper(client) });
    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.triggers).toEqual([TRIGGER]);
    expect(result.current.notWired).toBe(false);
    expect(triggersList).toHaveBeenCalledWith({ limit: 200 });
  });

  it("degrades to notWired on an UNIMPLEMENTED gateway", async () => {
    const { client } = makeMockClient({
      triggersList: async () => {
        throw new KxUnimplemented("triggers not wired");
      },
    });
    const { result } = renderHook(() => useListTriggers(), { wrapper: connectedWrapper(client) });
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.notWired).toBe(true);
  });
});

describe("useRegisterTrigger", () => {
  it("registers a trigger (name · kind · recipe · auth · secret ref)", async () => {
    const { client, triggersAdd } = makeMockClient();
    const { result } = renderHook(() => useRegisterTrigger(), {
      wrapper: connectedWrapper(client),
    });
    await act(async () => {
      await result.current.mutateAsync({
        name: "gh-push",
        kind: "webhook",
        recipeHandle: "kx/recipes/react",
        auth: "hmac_sha256",
        authSecretRef: "gh_hmac",
        scheduleSpec: "",
        enabled: true,
      });
    });
    expect(triggersAdd).toHaveBeenCalledWith({
      name: "gh-push",
      kind: "webhook",
      recipeHandle: "kx/recipes/react",
      auth: "hmac_sha256",
      authSecretRef: "gh_hmac",
      scheduleSpec: "",
      enabled: true,
    });
  });
});

describe("useTestTrigger / useFireTrigger / useDeregisterTrigger", () => {
  it("dry-run tests a trigger binding (fires nothing)", async () => {
    const { client, triggersTest } = makeMockClient({
      triggersTest: async () => ({ ok: true, detail: "binds" }),
    });
    const { result } = renderHook(() => useTestTrigger(), { wrapper: connectedWrapper(client) });
    await act(async () => {
      await result.current.mutateAsync({ name: "gh-push" });
    });
    expect(triggersTest).toHaveBeenCalledWith("gh-push", undefined);
  });

  it("fires a trigger (the inbound event)", async () => {
    const { client, triggersFire } = makeMockClient({
      triggersFire: async () => ({ instanceId: "cd".repeat(8), deduped: false }),
    });
    const { result } = renderHook(() => useFireTrigger(), { wrapper: connectedWrapper(client) });
    await act(async () => {
      const out = await result.current.mutateAsync({ name: "gh-push" });
      expect(out.instanceId).toBe("cd".repeat(8));
    });
    expect(triggersFire).toHaveBeenCalledWith("gh-push", undefined, undefined);
  });

  it("deregisters a trigger by name", async () => {
    const { client, triggersRemove } = makeMockClient();
    const { result } = renderHook(() => useDeregisterTrigger(), {
      wrapper: connectedWrapper(client),
    });
    await act(async () => {
      await result.current.mutateAsync("gh-push");
    });
    expect(triggersRemove).toHaveBeenCalledWith("gh-push");
  });
});
