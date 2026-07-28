/** POC-5d chat ORCHESTRATION hook — config pins vs interactive state, the naming
 *  state machine, autosave/rename/export, and what it passes into `useChat`.
 *  `useChat` itself is mocked (its I/O round-trip has its own spec); the sibling
 *  discovery hooks (recipes/models/context) run real against the mocked client. */

import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

const h = vi.hoisted(() => ({
  useChatOpts: [] as unknown[],
  send: vi.fn(async () => {}),
  reset: vi.fn(),
  loadThread: vi.fn(),
  thread: { messages: [] as unknown[] },
  saveChat: vi.fn(() => []),
  renameChat: vi.fn(() => []),
  download: vi.fn(),
}));

vi.mock("../../src/kx/use-chat", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/kx/use-chat")>();
  return {
    ...actual,
    useChat: (opts: unknown) => {
      h.useChatOpts.push(opts);
      return {
        thread: h.thread,
        busy: false,
        degraded: null,
        activeProjection: undefined,
        activeAssistantId: undefined,
        reactTurns: undefined,
        send: h.send,
        retry: vi.fn(async () => {}),
        cancel: vi.fn(),
        loadThread: h.loadThread,
        reset: h.reset,
      } as unknown as ReturnType<typeof actual.useChat>;
    },
  };
});

vi.mock("../../src/lib/chat-history", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/lib/chat-history")>();
  return { ...actual, saveChat: h.saveChat, renameChat: h.renameChat };
});

vi.mock("../../src/lib/download", () => ({ download: h.download }));

import { useChatController } from "../../src/components/chat/useChatController";
import { REACT_RECIPE_HANDLE } from "../../src/kx/use-chat";

const ENDPOINT = "http://127.0.0.1:50151";
const GEMMA = {
  modelId: "gemma",
  displayName: "Gemma",
  active: true,
  chatHandle: "kx/recipes/chat",
};
const DONE_MSG = { id: "m1", role: "user", text: "hello there", status: "done" };

function lastChatOpts(): Record<string, unknown> {
  return h.useChatOpts.at(-1) as Record<string, unknown>;
}

beforeEach(() => {
  localStorage.clear();
  h.useChatOpts.length = 0;
  h.thread = { messages: [] };
  h.send.mockClear();
  h.reset.mockClear();
  h.loadThread.mockClear();
  h.saveChat.mockClear();
  h.renameChat.mockClear();
  h.download.mockClear();
});

describe("useChatController — standalone (no config)", () => {
  it("drives read-only chat from defaults and prompts for a model on an empty serve", async () => {
    const { client } = makeMockClient();
    const { result } = renderHook(() => useChatController(), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.promptNoModel).toBe(true));
    expect(result.current.backingHandle).toBe("kx/recipes/chat");
    expect(result.current.agentTurn).toBe(false);
    expect(result.current.boundModel).toBeUndefined();
    expect(lastChatOpts()).toMatchObject({
      handle: "kx/recipes/chat",
      promptKey: "prompt",
      agentMode: false,
    });
    expect(lastChatOpts().modelId).toBeUndefined();
    expect(lastChatOpts().dataset).toBeUndefined();
    // Autosave defaults ON for the standalone route: the mount effect persists.
    expect(h.saveChat).toHaveBeenCalled();
  });

  it("the interactive dataset picker feeds the turn (and clears)", async () => {
    const { client } = makeMockClient();
    const { result } = renderHook(() => useChatController(), {
      wrapper: connectedWrapper(client),
    });
    act(() => result.current.setDataset("ops-notes"));
    expect(result.current.dataset).toBe("ops-notes");
    expect(lastChatOpts().dataset).toBe("ops-notes");
    act(() => result.current.setDataset(undefined));
    expect(result.current.dataset).toBeUndefined();
  });

  it("toggleContext adds then removes a pending bundle", async () => {
    const { client } = makeMockClient();
    const { result } = renderHook(() => useChatController(), {
      wrapper: connectedWrapper(client),
    });
    act(() => result.current.toggleContext("b1"));
    expect(result.current.pendingContext).toEqual(["b1"]);
    act(() => result.current.toggleContext("b1"));
    expect(result.current.pendingContext).toEqual([]);
  });
});

describe("useChatController — config pins (AppChat)", () => {
  const CONFIG = {
    backing: { handle: "kx/recipes/react-fs", promptKey: "goal" },
    modelId: "gemma",
    agentMode: true,
    dataset: "ops-notes",
    autosave: false,
    contextRefs: ["ctx-a"] as const,
  };

  it("pins win over interactive state and reach the turn intact", async () => {
    const { client } = makeMockClient({
      listRecipes: async () => [REACT_RECIPE_HANDLE, "kx/recipes/react-fs"],
      listModels: async () => [GEMMA],
    });
    const { result } = renderHook(() => useChatController(CONFIG), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.agentTurn).toBe(true));
    expect(result.current.backingHandle).toBe("kx/recipes/react-fs");
    expect(result.current.boundModel?.modelId).toBe("gemma");
    expect(lastChatOpts()).toMatchObject({
      handle: "kx/recipes/react-fs",
      promptKey: "goal",
      modelId: "gemma",
      agentMode: true,
      dataset: "ops-notes",
      contextRefs: ["ctx-a"],
    });
    // A config-pinned dataset ignores the interactive setter.
    act(() => result.current.setDataset("other"));
    expect(result.current.dataset).toBe("ops-notes");
    // An embedded App chat never writes the client-local history.
    expect(h.saveChat).not.toHaveBeenCalled();
  });

  it("agent mode stays OFF while the react recipe is not provisioned", async () => {
    const { client } = makeMockClient({ listModels: async () => [GEMMA] });
    const { result } = renderHook(() => useChatController({ agentMode: true }), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.boundModel?.modelId).toBe("gemma"));
    expect(result.current.agentTurn).toBe(false);
    expect(lastChatOpts().agentMode).toBe(false);
  });
});

describe("useChatController — the naming state machine", () => {
  it("auto-names from the first message of an unnamed chat", async () => {
    const { client } = makeMockClient();
    const { result } = renderHook(() => useChatController(), {
      wrapper: connectedWrapper(client),
    });
    act(() => result.current.sendWithAttachments("Ship the fix tonight"));
    expect(result.current.chatName).toBe("Ship the fix tonight");
    expect(h.send).toHaveBeenCalledWith("Ship the fix tonight", [], [], []);
  });

  it("setChatName does NOT claim the name — the next send still auto-names", async () => {
    const { client } = makeMockClient();
    const { result } = renderHook(() => useChatController(), {
      wrapper: connectedWrapper(client),
    });
    act(() => result.current.setChatName("Programmatic"));
    act(() => result.current.sendWithAttachments("Ship the fix tonight"));
    expect(result.current.chatName).toBe("Ship the fix tonight");
  });

  it("onChatNameInput DOES claim the name — auto-naming stands down", async () => {
    const { client } = makeMockClient();
    const { result } = renderHook(() => useChatController(), {
      wrapper: connectedWrapper(client),
    });
    act(() => result.current.onChatNameInput("My triage thread"));
    act(() => result.current.sendWithAttachments("Ship the fix tonight"));
    expect(result.current.chatName).toBe("My triage thread");
  });

  it("newChat resets the thread, the name, and the renamed claim", async () => {
    const { client } = makeMockClient();
    const { result } = renderHook(() => useChatController(), {
      wrapper: connectedWrapper(client),
    });
    act(() => result.current.onChatNameInput("Claimed"));
    act(() => result.current.newChat());
    expect(h.reset).toHaveBeenCalledTimes(1);
    expect(result.current.chatName).not.toBe("Claimed");
    // The claim is gone: the first send of the NEW chat auto-names again.
    act(() => result.current.sendWithAttachments("Fresh start"));
    expect(result.current.chatName).toBe("Fresh start");
  });

  it("loadSaved adopts the saved thread and its name as claimed", async () => {
    const { client } = makeMockClient();
    const { result } = renderHook(() => useChatController(), {
      wrapper: connectedWrapper(client),
    });
    const saved = {
      id: "saved-1",
      name: "Yesterday's triage",
      title: "hello",
      createdAt: 0,
      updatedAt: 0,
      messages: [DONE_MSG],
    };
    act(() => result.current.loadSaved(saved as never));
    expect(h.loadThread).toHaveBeenCalledWith([DONE_MSG]);
    expect(result.current.chatName).toBe("Yesterday's triage");
    // The saved name is a user claim — a follow-up send must not rename it.
    act(() => result.current.sendWithAttachments("and another thing"));
    expect(result.current.chatName).toBe("Yesterday's triage");
  });
});

describe("useChatController — export and commit", () => {
  it("exportChat is a no-op on an empty thread", async () => {
    const { client } = makeMockClient();
    const { result } = renderHook(() => useChatController(), {
      wrapper: connectedWrapper(client),
    });
    act(() => result.current.exportChat());
    expect(h.download).not.toHaveBeenCalled();
  });

  it("exportChat downloads the thread as JSON", async () => {
    h.thread = { messages: [DONE_MSG] };
    const { client } = makeMockClient();
    const { result } = renderHook(() => useChatController(), {
      wrapper: connectedWrapper(client),
    });
    act(() => result.current.exportChat());
    expect(h.download).toHaveBeenCalledTimes(1);
    expect(h.download.mock.calls[0]?.[2]).toBe("application/json");
  });

  it("commitName renames only a non-empty autosaved thread", async () => {
    const { client } = makeMockClient();
    const empty = renderHook(() => useChatController(), { wrapper: connectedWrapper(client) });
    act(() => empty.result.current.commitName());
    expect(h.renameChat).not.toHaveBeenCalled();

    h.thread = { messages: [DONE_MSG] };
    const { result } = renderHook(() => useChatController(), {
      wrapper: connectedWrapper(client),
    });
    act(() => result.current.onChatNameInput("Kept name"));
    act(() => result.current.commitName());
    expect(h.renameChat).toHaveBeenCalledWith(ENDPOINT, expect.any(String), "Kept name");
  });
});
