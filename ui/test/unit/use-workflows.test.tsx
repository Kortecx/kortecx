/**
 * The durable-Workflow hooks: the catalog + single-envelope reads, the
 * save (handle + caller-stated lifecycle ride the wire), the run (a Run-shaped
 * result carries BOTH anchors — never a waited Result), and the delete (the
 * cascade report surfaces verbatim).
 */

import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  useDeleteWorkflow,
  useRunWorkflow,
  useSaveWorkflow,
  useWorkflow,
  useWorkflows,
} from "../../src/kx/use-workflows";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

const SUMMARY = {
  handle: "workflows/local/digest",
  workflowRef: "ab".repeat(16),
  name: "Morning digest",
  version: "1",
  description: "Summarize the overnight items",
  tags: [],
  stepCount: 3,
  delivers: "",
  lifecycle: "",
};

describe("useWorkflows", () => {
  it("lists the caller's workflow catalog", async () => {
    const mock = makeMockClient({ listWorkflows: async () => [SUMMARY] });
    const { result } = renderHook(() => useWorkflows(), {
      wrapper: connectedWrapper(mock.client),
    });
    await waitFor(() => expect(result.current.workflows).toHaveLength(1));
    expect(result.current.workflows[0]?.handle).toBe("workflows/local/digest");
    expect(result.current.notWired).toBe(false);
  });
});

describe("useWorkflow", () => {
  it("fetches one stored envelope by handle; null stays null (uniform not-found)", async () => {
    const stored = {
      envelope: { schema: "kortecx.workflow/v1", name: "Morning digest" },
      workflowDigest: "cd".repeat(32),
      lifecycle: "draft",
      stepCount: 3,
    };
    const mock = makeMockClient({ getWorkflow: async () => stored });
    const { result } = renderHook(() => useWorkflow("workflows/local/digest"), {
      wrapper: connectedWrapper(mock.client),
    });
    await waitFor(() => expect(result.current.data).toEqual(stored));
    expect(mock.getWorkflow).toHaveBeenCalledWith("workflows/local/digest");
  });

  it("does not fire with a null handle", () => {
    const mock = makeMockClient();
    renderHook(() => useWorkflow(null), { wrapper: connectedWrapper(mock.client) });
    expect(mock.getWorkflow).not.toHaveBeenCalled();
  });
});

describe("useSaveWorkflow", () => {
  it("sends the envelope + handle + caller-stated lifecycle", async () => {
    const mock = makeMockClient({
      saveWorkflow: async () => ({
        workflowRef: "ab".repeat(16),
        handle: "workflows/local/digest",
        deduplicated: false,
      }),
    });
    const { result } = renderHook(() => useSaveWorkflow(), {
      wrapper: connectedWrapper(mock.client),
    });
    const envelope = { schema: "kortecx.workflow/v1", name: "Morning digest" };
    act(() => {
      result.current.mutate({ handle: "workflows/local/digest", envelope, lifecycle: "draft" });
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mock.saveWorkflow).toHaveBeenCalledWith(envelope, {
      handle: "workflows/local/digest",
      lifecycle: "draft",
    });
    expect(result.current.data?.handle).toBe("workflows/local/digest");
  });

  it("defaults the lifecycle to active (empty)", async () => {
    const mock = makeMockClient();
    const { result } = renderHook(() => useSaveWorkflow(), {
      wrapper: connectedWrapper(mock.client),
    });
    act(() => {
      result.current.mutate({ handle: "workflows/local/digest", envelope: {} });
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mock.saveWorkflow).toHaveBeenCalledWith(
      {},
      { handle: "workflows/local/digest", lifecycle: "" },
    );
  });
});

describe("useRunWorkflow", () => {
  it("returns BOTH run anchors from a Run-shaped result", async () => {
    const mock = makeMockClient();
    const { result } = renderHook(() => useRunWorkflow(), {
      wrapper: connectedWrapper(mock.client),
    });
    act(() => {
      result.current.mutate({ handle: "workflows/local/digest" });
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual({
      instanceId: "ab".repeat(16),
      reactChainSalt: "cd".repeat(16),
      terminalMoteId: "ef".repeat(32),
    });
  });

  it("rejects a waited-Result shape (no recipeFingerprint) rather than mis-anchoring", async () => {
    const mock = makeMockClient({ runWorkflow: async () => ({ text: "done" }) });
    const { result } = renderHook(() => useRunWorkflow(), {
      wrapper: connectedWrapper(mock.client),
    });
    act(() => {
      result.current.mutate({ handle: "workflows/local/digest" });
    });
    await waitFor(() => expect(result.current.isError).toBe(true));
  });
});

describe("useDeleteWorkflow", () => {
  it("surfaces the cascade report verbatim", async () => {
    const mock = makeMockClient({
      deleteWorkflow: async () => ({
        removed: true,
        branchUnbound: true,
        lockCleared: false,
        triggersRemoved: 2,
      }),
    });
    const { result } = renderHook(() => useDeleteWorkflow(), {
      wrapper: connectedWrapper(mock.client),
    });
    act(() => {
      result.current.mutate({ handle: "workflows/local/digest" });
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual({
      removed: true,
      branchUnbound: true,
      lockCleared: false,
      triggersRemoved: 2,
    });
    expect(mock.deleteWorkflow).toHaveBeenCalledWith("workflows/local/digest");
  });
});
