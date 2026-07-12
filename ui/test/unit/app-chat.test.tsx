/**
 * POC-5d: the embedded App chat. Mounts the shared ChatSurface with the
 * interactive chrome OFF (no model/dataset pickers, no history actions, no
 * Chat/Agent toggle) — the App fixes the recipe + grounding; a user just types
 * and sends. We mock the controller so the test asserts the SURFACE config
 * (flags off) + that send wiring fires through the fixed recipe.
 */

import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ChatController } from "../../src/components/chat/useChatController";
import { EMPTY_THREAD } from "../../src/lib/chat-thread";

const sendWithAttachments = vi.fn();

// A minimal controller stub — enough to render ChatSurface deterministically.
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
    chatName: "App chat",
    setChatName: vi.fn(),
    onChatNameInput: vi.fn(),
    commitName: vi.fn(),
    newChat: vi.fn(),
    loadSaved: vi.fn(),
    exportChat: vi.fn(),
    sendWithAttachments,
  };
}

vi.mock("../../src/components/chat/useChatController", () => ({
  useChatController: () => makeController(),
}));

vi.mock("../../src/kx/connection-context", () => ({
  useConnection: () => ({ endpoint: "http://127.0.0.1:50151" }),
}));

import { AppChat } from "../../src/components/chat/AppChat";

afterEach(() => {
  sendWithAttachments.mockReset();
});

describe("AppChat (POC-5d embedded App chat)", () => {
  it("mounts with its own section testid", () => {
    render(<AppChat recipeHandle="apps/local/demo" />);
    expect(screen.getByTestId("app-chat")).toBeInTheDocument();
  });

  it("hides the interactive chrome — no pickers, history, or mode toggle", () => {
    render(<AppChat recipeHandle="apps/local/demo" />);
    // No model/dataset pickers, no history/new/export actions, no agent toggle,
    // no settings panel — the App fixes everything.
    expect(screen.queryByTestId("model-picker")).toBeNull();
    expect(screen.queryByTestId("model-picker-empty")).toBeNull();
    expect(screen.queryByTestId("chat-history-toggle")).toBeNull();
    expect(screen.queryByTestId("chat-new")).toBeNull();
    expect(screen.queryByTestId("chat-export")).toBeNull();
    expect(screen.queryByTestId("chat-mode")).toBeNull();
    expect(screen.queryByTestId("chat-name")).toBeNull();
  });

  it("renders the App-scoped header + the composer (send via the fixed recipe)", () => {
    render(<AppChat recipeHandle="apps/local/demo" />);
    expect(screen.getByTestId("app-chat-head")).toBeInTheDocument();
    expect(screen.getByText(/apps\/local\/demo/)).toBeInTheDocument();
    // The composer is present (the user can type + send a turn).
    expect(screen.getByTestId("composer")).toBeInTheDocument();
    // Drive a send via the headless editor textarea + the send button.
    const editor = screen.getByTestId("composer-input");
    fireEvent.change(editor, { target: { value: "hello app" } });
    fireEvent.click(screen.getByTestId("send"));
    expect(sendWithAttachments).toHaveBeenCalledWith("hello app");
  });
});
