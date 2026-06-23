import { KxUnimplemented } from "@kortecx/sdk/web";
import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  useContextBundles,
  useDeleteContextBundle,
  useEditBundleDescription,
  useEditContextItem,
  usePutContextBundle,
  useRemoveContextItem,
  useRenameContextItem,
} from "../../src/kx/use-context-bundles";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

const BUNDLE = {
  bundleRef: "ab".repeat(8),
  handle: "team/ctx/spec",
  description: "the spec",
  itemCount: 1,
  items: [{ name: "design.md", contentRef: "cd".repeat(32), mediaType: "text/markdown" }],
};

describe("useContextBundles", () => {
  it("lists the party's bundles", async () => {
    const { client } = makeMockClient({ listContextBundles: async () => [BUNDLE] });
    const { result } = renderHook(() => useContextBundles(), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.bundles).toEqual([BUNDLE]);
    expect(result.current.notWired).toBe(false);
  });

  it("degrades to notWired on an UNIMPLEMENTED gateway", async () => {
    const { client } = makeMockClient({
      listContextBundles: async () => {
        throw new KxUnimplemented("context bundles not wired");
      },
    });
    const { result } = renderHook(() => useContextBundles(), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.notWired).toBe(true);
  });
});

describe("usePutContextBundle / useDeleteContextBundle", () => {
  it("upserts a bundle with its items + description", async () => {
    const { client, putContextBundle } = makeMockClient();
    const { result } = renderHook(() => usePutContextBundle(), {
      wrapper: connectedWrapper(client),
    });
    await act(async () => {
      await result.current.mutateAsync({
        handle: "team/ctx/spec",
        items: [{ name: "design.md", contentRef: "cd".repeat(32) }],
        description: "the spec",
      });
    });
    expect(putContextBundle).toHaveBeenCalledWith(
      "team/ctx/spec",
      [{ name: "design.md", contentRef: "cd".repeat(32) }],
      { description: "the spec" },
    );
  });

  it("deletes a bundle by handle", async () => {
    const { client, deleteContextBundle } = makeMockClient();
    const { result } = renderHook(() => useDeleteContextBundle(), {
      wrapper: connectedWrapper(client),
    });
    await act(async () => {
      await result.current.mutateAsync({ handle: "team/ctx/spec" });
    });
    expect(deleteContextBundle).toHaveBeenCalledWith("team/ctx/spec");
  });
});

describe("POC-2 context-edit mutations", () => {
  it("edits an item body via the SDK (encoded bytes + stale-base guard ref)", async () => {
    const { client, editContextItem } = makeMockClient();
    const { result } = renderHook(() => useEditContextItem(), {
      wrapper: connectedWrapper(client),
    });
    await act(async () => {
      await result.current.mutateAsync({
        handle: "team/ctx/spec",
        itemIndex: 0,
        text: "new body",
        mediaType: "text/markdown",
        expectBundleRef: "ab".repeat(8),
      });
    });
    expect(editContextItem).toHaveBeenCalledWith(
      "team/ctx/spec",
      0,
      new TextEncoder().encode("new body"),
      { mediaType: "text/markdown", expectBundleRef: "ab".repeat(8) },
    );
  });

  it("removes an item via the SDK", async () => {
    const { client, removeContextItem } = makeMockClient();
    const { result } = renderHook(() => useRemoveContextItem(), {
      wrapper: connectedWrapper(client),
    });
    await act(async () => {
      await result.current.mutateAsync({ handle: "team/ctx/spec", itemIndex: 1 });
    });
    expect(removeContextItem).toHaveBeenCalledWith("team/ctx/spec", 1, {
      expectBundleRef: undefined,
    });
  });

  it("renames an item with a guarded re-upsert (re-read → put with the new name)", async () => {
    const { client, getContextBundle, putContextBundle } = makeMockClient({
      getContextBundle: async () => BUNDLE,
    });
    const { result } = renderHook(() => useRenameContextItem(), {
      wrapper: connectedWrapper(client),
    });
    await act(async () => {
      await result.current.mutateAsync({
        handle: "team/ctx/spec",
        itemIndex: 0,
        newName: "renamed.md",
        expectBundleRef: BUNDLE.bundleRef,
      });
    });
    expect(getContextBundle).toHaveBeenCalledWith("team/ctx/spec");
    expect(putContextBundle).toHaveBeenCalledWith(
      "team/ctx/spec",
      [{ name: "renamed.md", contentRef: "cd".repeat(32), mediaType: "text/markdown" }],
      { description: "the spec" },
    );
  });

  it("edits a bundle description with a guarded re-upsert (items unchanged)", async () => {
    const { client, putContextBundle } = makeMockClient({
      getContextBundle: async () => BUNDLE,
    });
    const { result } = renderHook(() => useEditBundleDescription(), {
      wrapper: connectedWrapper(client),
    });
    await act(async () => {
      await result.current.mutateAsync({
        handle: "team/ctx/spec",
        description: "a better spec",
        expectBundleRef: BUNDLE.bundleRef,
      });
    });
    expect(putContextBundle).toHaveBeenCalledWith(
      "team/ctx/spec",
      [{ name: "design.md", contentRef: "cd".repeat(32), mediaType: "text/markdown" }],
      { description: "a better spec" },
    );
  });

  it("the stale-base guard refuses a re-upsert when the bundle changed under the editor", async () => {
    const { client, putContextBundle } = makeMockClient({
      // The current manifest has a DIFFERENT bundleRef than the one the user viewed.
      getContextBundle: async () => ({ ...BUNDLE, bundleRef: "99".repeat(8) }),
    });
    const { result } = renderHook(() => useRenameContextItem(), {
      wrapper: connectedWrapper(client),
    });
    await act(async () => {
      await expect(
        result.current.mutateAsync({
          handle: "team/ctx/spec",
          itemIndex: 0,
          newName: "x.md",
          expectBundleRef: BUNDLE.bundleRef, // stale
        }),
      ).rejects.toThrow(/changed since/);
    });
    expect(putContextBundle).not.toHaveBeenCalled();
  });
});
