import { KxUnimplemented } from "@kortecx/sdk/web";
import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  useContextBundles,
  useDeleteContextBundle,
  usePutContextBundle,
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
