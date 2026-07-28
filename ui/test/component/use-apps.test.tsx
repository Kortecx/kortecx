/** POC-4/5d App-catalog hooks — inventory, envelope fetch, run/save/export/import/
 *  clone/delete, and the Template tag toggle. The client is mocked. */

import { KxUnimplemented } from "@kortecx/sdk/web";
import { QueryClient } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { queryKeys } from "../../src/kx/query-keys";
import {
  TEMPLATE_TAG,
  useApp,
  useApps,
  useCloneApp,
  useDeleteApp,
  useExportAppBundle,
  useImportApp,
  useRunApp,
  useSaveApp,
  useToggleTemplate,
} from "../../src/kx/use-apps";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

const ENDPOINT = "http://127.0.0.1:50151";

const SUMMARY = {
  handle: "triage-bot",
  appRef: "ab".repeat(32),
  name: "Triage bot",
  version: 1,
  description: "",
  tags: [],
  stepCount: 1,
  locked: false,
  kind: "",
  mode: "",
};

afterEach(() => {
  vi.restoreAllMocks();
});

describe("useApps", () => {
  it("lists the App inventory", async () => {
    const { client, listApps } = makeMockClient({ listApps: async () => [SUMMARY] });
    const { result } = renderHook(() => useApps(), { wrapper: connectedWrapper(client) });
    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.apps).toEqual([SUMMARY]);
    expect(result.current.notWired).toBe(false);
    expect(listApps).toHaveBeenCalledTimes(1);
  });

  it("degrades to notWired on an UNIMPLEMENTED gateway", async () => {
    const { client } = makeMockClient({
      listApps: async () => {
        throw new KxUnimplemented("apps not wired");
      },
    });
    const { result } = renderHook(() => useApps(), { wrapper: connectedWrapper(client) });
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.notWired).toBe(true);
    expect(result.current.apps).toEqual([]);
  });
});

describe("useApp", () => {
  it("fetches one App's stored envelope by handle", async () => {
    const stored = {
      summary: SUMMARY,
      envelope: { name: "Triage bot" },
      appDigest: "cd".repeat(32),
      sourceDigest: "",
    };
    const { client, getApp } = makeMockClient({ getApp: async () => stored });
    const { result } = renderHook(() => useApp("triage-bot"), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.data).toEqual(stored);
    expect(getApp).toHaveBeenCalledWith("triage-bot");
  });

  it("never fires for a null handle (disabled query)", async () => {
    const { client, getApp } = makeMockClient();
    const { result } = renderHook(() => useApp(null), { wrapper: connectedWrapper(client) });
    await act(async () => {});
    expect(result.current.fetchStatus).toBe("idle");
    expect(getApp).not.toHaveBeenCalled();
  });
});

describe("useRunApp", () => {
  it("maps the Run handle's scoping keys (salt + terminal mote)", async () => {
    const { client, runApp } = makeMockClient();
    const { result } = renderHook(() => useRunApp(), { wrapper: connectedWrapper(client) });
    await act(async () => {
      const out = await result.current.mutateAsync({
        handle: "triage-bot",
        args: { topic: "billing" },
        requireApproval: true,
      });
      expect(out).toEqual({
        instanceId: "ab".repeat(16),
        reactChainSalt: "cd".repeat(16),
        terminalMoteId: "ef".repeat(32),
      });
    });
    expect(runApp).toHaveBeenCalledWith("triage-bot", {
      args: { topic: "billing" },
      requireApproval: true,
    });
  });

  it("rejects a waited Result shape (the no-wait guard)", async () => {
    // A `wait`-style Result has no recipeFingerprint — the guard that caught the
    // 4-arg-ctor regression must throw rather than return empty scoping keys.
    const { client } = makeMockClient({
      runApp: async () => ({ instanceId: "ab".repeat(16), wait: async () => ({}) }),
    });
    const { result } = renderHook(() => useRunApp(), { wrapper: connectedWrapper(client) });
    await act(async () => {
      await expect(result.current.mutateAsync({ handle: "triage-bot" })).rejects.toThrow(
        "unexpected runApp result",
      );
    });
  });
});

describe("useSaveApp", () => {
  it("saves the envelope and invalidates the App + inventory caches", async () => {
    const spy = vi.spyOn(QueryClient.prototype, "invalidateQueries");
    const { client, saveApp } = makeMockClient({
      saveApp: async () => ({ appRef: "ab".repeat(32), handle: "triage-bot", deduplicated: true }),
    });
    const { result } = renderHook(() => useSaveApp(), { wrapper: connectedWrapper(client) });
    const envelope = { name: "Triage bot", steps: [] };
    await act(async () => {
      const out = await result.current.mutateAsync({ handle: "triage-bot", envelope });
      expect(out).toEqual({ appRef: "ab".repeat(32), deduplicated: true });
    });
    expect(saveApp).toHaveBeenCalledWith(envelope, { handle: "triage-bot" });
    expect(spy).toHaveBeenCalledWith({ queryKey: queryKeys.app(ENDPOINT, "triage-bot") });
    expect(spy).toHaveBeenCalledWith({ queryKey: queryKeys.apps(ENDPOINT) });
  });
});

describe("useExportAppBundle / useImportApp / useCloneApp / useDeleteApp", () => {
  it("exports a bundle (withData defaults to false)", async () => {
    const { client, exportAppBundle } = makeMockClient({
      exportAppBundle: async () => "bundle-wire",
    });
    const { result } = renderHook(() => useExportAppBundle(), {
      wrapper: connectedWrapper(client),
    });
    await act(async () => {
      expect(await result.current.mutateAsync({ handle: "triage-bot" })).toBe("bundle-wire");
    });
    expect(exportAppBundle).toHaveBeenCalledWith("triage-bot", { withData: false });
  });

  it("imports a bundle (force defaults to false) and invalidates the inventory", async () => {
    const spy = vi.spyOn(QueryClient.prototype, "invalidateQueries");
    const { client, importApp } = makeMockClient();
    const { result } = renderHook(() => useImportApp(), { wrapper: connectedWrapper(client) });
    await act(async () => {
      const out = await result.current.mutateAsync({ bundle: "bundle-wire" });
      expect(out).toEqual({ handle: "app" });
    });
    expect(importApp).toHaveBeenCalledWith("bundle-wire", { force: false });
    expect(spy).toHaveBeenCalledWith({ queryKey: queryKeys.apps(ENDPOINT) });
  });

  it("clones under a new name", async () => {
    const { client, cloneApp } = makeMockClient({
      cloneApp: async () => ({ appRef: "ab".repeat(32), handle: "copy", deduplicated: false }),
    });
    const { result } = renderHook(() => useCloneApp(), { wrapper: connectedWrapper(client) });
    await act(async () => {
      const out = await result.current.mutateAsync({ handle: "triage-bot", newname: "copy" });
      expect(out).toEqual({ handle: "copy" });
    });
    expect(cloneApp).toHaveBeenCalledWith("triage-bot", "copy");
  });

  it("reports what the delete cascade actually did", async () => {
    const outcome = {
      removed: true,
      branchUnbound: true,
      lockCleared: false,
      hostedStopped: false,
      triggersRemoved: 2,
    };
    const { client, deleteApp } = makeMockClient({ deleteApp: async () => outcome });
    const { result } = renderHook(() => useDeleteApp(), { wrapper: connectedWrapper(client) });
    await act(async () => {
      expect(await result.current.mutateAsync({ handle: "triage-bot" })).toEqual(outcome);
    });
    expect(deleteApp).toHaveBeenCalledWith("triage-bot");
  });
});

describe("useToggleTemplate", () => {
  const stored = (envelope: Record<string, unknown>) => ({
    summary: SUMMARY,
    envelope,
    appDigest: "cd".repeat(32),
    sourceDigest: "",
  });

  it("rejects when the App does not exist", async () => {
    const { client } = makeMockClient({ getApp: async () => null });
    const { result } = renderHook(() => useToggleTemplate(), {
      wrapper: connectedWrapper(client),
    });
    await act(async () => {
      await expect(result.current.mutateAsync({ handle: "gone" })).rejects.toThrow("app not found");
    });
  });

  it("adds the reserved tag to an untagged envelope", async () => {
    const { client, saveApp } = makeMockClient({
      getApp: async () => stored({ name: "Triage bot" }),
    });
    const { result } = renderHook(() => useToggleTemplate(), {
      wrapper: connectedWrapper(client),
    });
    await act(async () => {
      const out = await result.current.mutateAsync({ handle: "triage-bot" });
      expect(out).toEqual({ isTemplate: true });
    });
    expect(saveApp).toHaveBeenCalledWith(
      { name: "Triage bot", tags: [TEMPLATE_TAG] },
      {
        handle: "triage-bot",
      },
    );
  });

  it("removing the last tag drops the tags key entirely", async () => {
    const { client, saveApp } = makeMockClient({
      getApp: async () => stored({ name: "Triage bot", tags: [TEMPLATE_TAG] }),
    });
    const { result } = renderHook(() => useToggleTemplate(), {
      wrapper: connectedWrapper(client),
    });
    await act(async () => {
      const out = await result.current.mutateAsync({ handle: "triage-bot" });
      expect(out).toEqual({ isTemplate: false });
    });
    const savedEnv = saveApp.mock.calls[0]?.[0] as Record<string, unknown>;
    expect("tags" in savedEnv).toBe(false);
  });

  it("removing the tag preserves the App's other tags", async () => {
    const { client, saveApp } = makeMockClient({
      getApp: async () => stored({ name: "Triage bot", tags: ["support", TEMPLATE_TAG] }),
    });
    const { result } = renderHook(() => useToggleTemplate(), {
      wrapper: connectedWrapper(client),
    });
    await act(async () => {
      const out = await result.current.mutateAsync({ handle: "triage-bot" });
      expect(out).toEqual({ isTemplate: false });
    });
    expect(saveApp).toHaveBeenCalledWith(
      { name: "Triage bot", tags: ["support"] },
      {
        handle: "triage-bot",
      },
    );
  });
});
