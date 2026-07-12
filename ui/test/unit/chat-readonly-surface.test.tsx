/**
 * PR-A: the standalone New Chat is READ-ONLY, RAG-grounded — it shows the grounding
 * bar (dataset + context files) and EXCLUDES the mutate path: no Agent-task toggle,
 * no composer Tools/Context/Dataset categories. We mock the controller (deterministic
 * state) but render through a connected wrapper so the surface's sub-hooks are real.
 */

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatController } from "../../src/components/chat/useChatController";
import { EMPTY_THREAD } from "../../src/lib/chat-thread";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

function makeController(): ChatController {
  return {
    chat: {
      thread: EMPTY_THREAD,
      busy: false,
      degraded: null,
      activeProjection: undefined,
      activeAssistantId: undefined,
      reactTurns: undefined,
      send: vi.fn(async () => {}),
      retry: vi.fn(async () => {}),
      cancel: vi.fn(),
      loadThread: vi.fn(),
      reset: vi.fn(),
    },
    settings: {
      handle: "kx/recipes/chat",
      promptKey: "prompt",
      showThinking: true,
      showReasoning: true,
      autoscroll: true,
    },
    updateSettings: vi.fn(),
    agentTurn: false,
    dataset: undefined,
    setDataset: vi.fn(),
    backingHandle: "kx/recipes/chat",
    promptNoModel: false,
    attach: {
      attachments: [],
      uploading: false,
      addFiles: vi.fn(),
      remove: vi.fn(),
      clear: vi.fn(),
    } as unknown as ChatController["attach"],
    contextBundles: {
      bundles: [],
      notWired: false,
    } as unknown as ChatController["contextBundles"],
    pendingContext: [],
    toggleContext: vi.fn(),
    chatName: "New chat",
    setChatName: vi.fn(),
    onChatNameInput: vi.fn(),
    commitName: vi.fn(),
    newChat: vi.fn(),
    loadSaved: vi.fn(),
    exportChat: vi.fn(),
    sendWithAttachments: vi.fn(),
  };
}

vi.mock("../../src/components/chat/useChatController", () => ({
  useChatController: () => makeController(),
}));

import { ChatPanel } from "../../src/components/chat/ChatPanel";

function renderPanel() {
  const { client } = makeMockClient();
  return render(<ChatPanel />, { wrapper: connectedWrapper(client) });
}

describe("New Chat read-only RAG surface (PR-A)", () => {
  it("mounts the frozen section + the grounding bar, and has no Agent toggle", () => {
    renderPanel();
    expect(screen.getByTestId("chat-panel")).toBeInTheDocument();
    expect(screen.getByTestId("chat-grounding")).toBeInTheDocument();
    // The mutate-capable agentic toggle is gone from the read-only chat.
    expect(screen.queryByTestId("chat-mode")).toBeNull();
  });

  it("excludes the Tools / Context / Dataset categories from the attach menu (read-only)", () => {
    renderPanel();
    fireEvent.click(screen.getByTestId("attach-btn"));
    expect(screen.getByTestId("attach-menu")).toBeInTheDocument();
    expect(screen.getByTestId("attach-upload")).toBeInTheDocument();
    expect(screen.getByTestId("attach-blueprint")).toBeDisabled();
    // No mutate-capable tools, no in-menu context (moved to the bar), no dataset placeholder.
    expect(screen.queryByTestId("attach-tool-group")).toBeNull();
    expect(screen.queryByTestId("attach-context-group")).toBeNull();
    expect(screen.queryByTestId("attach-dataset")).toBeNull();
  });

  it("offers first-class context selection in the grounding bar (honest empty)", () => {
    renderPanel();
    fireEvent.click(screen.getByTestId("chat-grounding-add"));
    expect(screen.getByTestId("chat-grounding-menu")).toBeInTheDocument();
    expect(screen.getByTestId("chat-grounding-empty")).toBeInTheDocument();
  });
});
