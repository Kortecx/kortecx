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

/** One attachment riding a user message (Batch A): the SERVER-derived upload
 *  ref + advisory display fields. `objectUrl` is the session-local `blob:`
 *  preview of the user's own picked file (never untrusted server bytes). */
export interface MessageAttachment {
  readonly ref: string;
  readonly filename: string;
  readonly mediaType: string;
  readonly objectUrl?: string;
}

export interface ChatMessage {
  readonly id: string;
  readonly role: ChatRole;
  readonly text: string;
  readonly status: ChatStatus;
  /** The uploads attached to a user message (display + the vision arg source). */
  readonly attachments?: readonly MessageAttachment[];
  /** PR-7b: the context-bundle handles attached to a user turn (display + the
   *  retry source). Threaded into the turn's Invoke `context`; a different attached
   *  context ⇒ a different, independently-cached run. */
  readonly context?: readonly string[];
  /** The tools (`${name}@${version}`) attached to a user turn (display + the retry
   *  source). A non-empty set routes the turn to a bounded agentic tool loop. */
  readonly tools?: readonly string[];
  /** The paired user message id on an assistant turn (the retry join key). */
  readonly forUserId?: string;
  /** The run backing an assistant turn (set once Invoke returns). */
  readonly instanceId?: string;
  readonly terminalMoteId?: string;
  /** PR-4.2 (T-STREAM1): the in-flight agent chain's LIVE reasoning text, streamed
   *  out-of-band while `status === "thinking"`. Display-only + advisory — cleared
   *  when the committed answer lands (`turn_done`); never persisted as the answer.
   *  (Simple/vision turns stream straight into `text` instead.) */
  readonly streamingReasoning?: string;
  /** Set when the turn fails (Invoke error, failed terminal Mote, or fetch error). */
  readonly error?: UiError;
}

export interface ChatThread {
  readonly messages: readonly ChatMessage[];
}

export const EMPTY_THREAD: ChatThread = { messages: [] };

export type ChatAction =
  | {
      type: "user_send";
      userId: string;
      assistantId: string;
      text: string;
      attachments?: readonly MessageAttachment[];
      context?: readonly string[];
      tools?: readonly string[];
    }
  | { type: "turn_started"; assistantId: string; instanceId: string; terminalMoteId: string }
  | { type: "turn_thinking"; assistantId: string }
  /** PR-4.2 (T-STREAM1): the ADVISORY live token text. `target: "answer"` streams
   *  into the bubble (simple/vision); `target: "reasoning"` streams into the agent
   *  trace line (so a tool turn's raw envelope never masquerades as the answer).
   *  Ignored once the turn leaves `thinking` (the committed fact is the authority). */
  | { type: "token_streamed"; assistantId: string; text: string; target: "answer" | "reasoning" }
  | { type: "turn_done"; assistantId: string; text: string }
  | { type: "turn_failed"; assistantId: string; error: UiError }
  /** Re-dispatch a FAILED turn with its identical args (Batch A idempotent-run
   *  UX: content-addressed refs + server dedup make the re-run safe). */
  | { type: "turn_retry"; assistantId: string }
  /** Restore a saved thread (chat history) — replaces the whole message list. */
  | { type: "load_thread"; messages: readonly ChatMessage[] }
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
        attachments: action.attachments,
        context: action.context,
        tools: action.tools,
      };
      const assistant: ChatMessage = {
        id: action.assistantId,
        role: "assistant",
        text: "",
        status: "pending",
        forUserId: action.userId,
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
    case "token_streamed":
      // Only a THINKING turn accepts live tokens — a late chunk after the
      // committed answer landed (`turn_done`) must NEVER clobber the authority.
      return patch(state, action.assistantId, (m) => {
        if (m.status !== "thinking") {
          return m;
        }
        return action.target === "reasoning"
          ? { ...m, streamingReasoning: action.text }
          : { ...m, text: action.text };
      });
    case "turn_done":
      return patch(state, action.assistantId, (m) => ({
        ...m,
        status: "done",
        text: action.text,
        // The committed answer supersedes the live reasoning trace.
        streamingReasoning: undefined,
      }));
    case "turn_failed":
      return patch(state, action.assistantId, (m) => ({
        ...m,
        status: "failed",
        error: action.error,
      }));
    case "turn_retry":
      // Only a FAILED turn re-arms (a done/in-flight turn is not retryable).
      return patch(state, action.assistantId, (m) =>
        m.status === "failed"
          ? { ...m, status: "pending", error: undefined, text: "", streamingReasoning: undefined }
          : m,
      );
    case "load_thread":
      return { messages: [...action.messages] };
    case "reset":
      return EMPTY_THREAD;
    default:
      return state;
  }
}

/** The user message a FAILED assistant turn replays (the retry args source). */
export function retrySource(
  state: ChatThread,
  assistantId: string,
): {
  text: string;
  attachments: readonly MessageAttachment[];
  context: readonly string[];
  tools: readonly string[];
} | null {
  const assistant = state.messages.find((m) => m.id === assistantId);
  if (!assistant || assistant.role !== "assistant" || assistant.status !== "failed") {
    return null;
  }
  const user = state.messages.find((m) => m.id === assistant.forUserId);
  if (!user) {
    return null;
  }
  return {
    text: user.text,
    attachments: user.attachments ?? [],
    context: user.context ?? [],
    tools: user.tools ?? [],
  };
}

/** True while any assistant turn is still in flight (composer stays disabled). */
export function isTurnInFlight(state: ChatThread): boolean {
  return state.messages.some(
    (m) => m.role === "assistant" && (m.status === "pending" || m.status === "thinking"),
  );
}
