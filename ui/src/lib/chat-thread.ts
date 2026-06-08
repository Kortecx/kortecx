/**
 * The chat thread state model + reducer — pure, illegal-states-minimized, fully
 * unit-testable. The `use-chat` hook owns the I/O (Invoke → poll → GetContent) and
 * only ever dispatches these actions; the reducer never touches the network.
 *
 * One assistant turn is backed by one runtime run: `instanceId` + `terminalMoteId`
 * (the authoritative poll-stop signal) ride on the assistant message so the UI can
 * render that turn's DAG-of-thought and re-open it later.
 */

import type { UiError } from "../kx/errors";

export type ChatRole = "user" | "assistant";
/** pending = invoking; thinking = run in flight; done/failed = terminal. */
export type ChatStatus = "pending" | "thinking" | "done" | "failed";

export interface ChatMessage {
  readonly id: string;
  readonly role: ChatRole;
  readonly text: string;
  readonly status: ChatStatus;
  /** The run backing an assistant turn (set once Invoke returns). */
  readonly instanceId?: string;
  readonly terminalMoteId?: string;
  /** Set when the turn fails (Invoke error, failed terminal Mote, or fetch error). */
  readonly error?: UiError;
}

export interface ChatThread {
  readonly messages: readonly ChatMessage[];
}

export const EMPTY_THREAD: ChatThread = { messages: [] };

export type ChatAction =
  | { type: "user_send"; userId: string; assistantId: string; text: string }
  | { type: "turn_started"; assistantId: string; instanceId: string; terminalMoteId: string }
  | { type: "turn_thinking"; assistantId: string }
  | { type: "turn_done"; assistantId: string; text: string }
  | { type: "turn_failed"; assistantId: string; error: UiError }
  | { type: "reset" };

/** Replace the message with `id` via `fn` (identity if not found). */
function patch(state: ChatThread, id: string, fn: (m: ChatMessage) => ChatMessage): ChatThread {
  return { messages: state.messages.map((m) => (m.id === id ? fn(m) : m)) };
}

export function chatReducer(state: ChatThread, action: ChatAction): ChatThread {
  switch (action.type) {
    case "user_send": {
      const user: ChatMessage = {
        id: action.userId,
        role: "user",
        text: action.text,
        status: "done",
      };
      const assistant: ChatMessage = {
        id: action.assistantId,
        role: "assistant",
        text: "",
        status: "pending",
      };
      return { messages: [...state.messages, user, assistant] };
    }
    case "turn_started":
      return patch(state, action.assistantId, (m) => ({
        ...m,
        status: "thinking",
        instanceId: action.instanceId,
        terminalMoteId: action.terminalMoteId,
      }));
    case "turn_thinking":
      return patch(state, action.assistantId, (m) =>
        m.status === "pending" ? { ...m, status: "thinking" } : m,
      );
    case "turn_done":
      return patch(state, action.assistantId, (m) => ({
        ...m,
        status: "done",
        text: action.text,
      }));
    case "turn_failed":
      return patch(state, action.assistantId, (m) => ({
        ...m,
        status: "failed",
        error: action.error,
      }));
    case "reset":
      return EMPTY_THREAD;
    default:
      return state;
  }
}

/** True while any assistant turn is still in flight (composer stays disabled). */
export function isTurnInFlight(state: ChatThread): boolean {
  return state.messages.some(
    (m) => m.role === "assistant" && (m.status === "pending" || m.status === "thinking"),
  );
}
