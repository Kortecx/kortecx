/**
 * Client-local chat history backed by localStorage (the `recent-runs.ts`
 * pattern): keyed PER ENDPOINT so switching gateways never mixes histories,
 * bounded, fail-closed (a corrupt/unavailable store yields an empty list).
 *
 * Per the build-out plan's open-question-5 decision this is PRESENTATION state
 * — client-local now; durable server-side sessions are a future sidecar batch.
 * Attachment `objectUrl`s are STRIPPED on save (`blob:` URLs die with the
 * document); a restored image preview re-resolves through the uploads scope
 * (`use-upload-preview`). In-flight turns are downgraded to `failed` on save so
 * a restored thread re-arms through the existing retry affordance instead of
 * spinning forever.
 */

import type { ChatMessage } from "./chat-thread";

export interface SavedChat {
  readonly id: string;
  /** Display title — the first user message, truncated. */
  readonly title: string;
  readonly createdAt: number;
  readonly updatedAt: number;
  readonly messages: readonly ChatMessage[];
}

/** Keep the history bounded (newest-updated first). */
const MAX_CHATS = 30;
const TITLE_MAX = 64;

/** Fired on `window` whenever the persisted history changes (same-tab refresh —
 *  the browser `storage` event only fires in OTHER tabs). */
export const CHATS_CHANGED_EVENT = "kortecx:chats-changed";

function notifyChatsChanged(): void {
  try {
    window.dispatchEvent(new Event(CHATS_CHANGED_EVENT));
  } catch {
    /* non-browser env */
  }
}

function keyFor(endpoint: string): string {
  return `kortecx.ui.chats:${endpoint}`;
}

/** The display title for a thread: its first user message, truncated. */
export function chatTitle(messages: readonly ChatMessage[]): string {
  const first = messages.find((m) => m.role === "user")?.text ?? "(empty chat)";
  const oneLine = first.replace(/\s+/g, " ").trim();
  return oneLine.length > TITLE_MAX ? `${oneLine.slice(0, TITLE_MAX)}…` : oneLine;
}

/** A message as persisted: no `blob:` object URLs, no in-flight statuses. */
function sanitizeMessage(m: ChatMessage): ChatMessage {
  const attachments = m.attachments?.map(({ objectUrl: _drop, ...rest }) => rest);
  const inFlight = m.status === "pending" || m.status === "thinking";
  return {
    ...m,
    attachments,
    // An interrupted turn restores as FAILED (re-armable via retry) — never as
    // a forever-spinner.
    status: inFlight ? "failed" : m.status,
    error: inFlight
      ? {
          code: "interrupted",
          kind: "retry",
          title: "Interrupted",
          message: "This turn was in flight when the chat was saved — retry to re-run it.",
          retryable: true,
        }
      : m.error,
  };
}

export function loadChats(endpoint: string): SavedChat[] {
  try {
    const raw = localStorage.getItem(keyFor(endpoint));
    if (raw === null) {
      return [];
    }
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }
    return parsed.filter(isSavedChat).slice(0, MAX_CHATS);
  } catch {
    return [];
  }
}

/** Upsert a chat by id (newest-updated first, bounded), return the list.
 *  An EMPTY thread is a no-op (nothing worth listing). */
export function saveChat(
  endpoint: string,
  id: string,
  messages: readonly ChatMessage[],
  now: number = Date.now(),
): SavedChat[] {
  if (messages.length === 0) {
    return loadChats(endpoint);
  }
  const existing = loadChats(endpoint);
  const prior = existing.find((c) => c.id === id);
  const next: SavedChat = {
    id,
    title: chatTitle(messages),
    createdAt: prior?.createdAt ?? now,
    updatedAt: now,
    messages: messages.map(sanitizeMessage),
  };
  const list = [next, ...existing.filter((c) => c.id !== id)].slice(0, MAX_CHATS);
  try {
    localStorage.setItem(keyFor(endpoint), JSON.stringify(list));
  } catch {
    /* best-effort (quota/private mode) */
  }
  notifyChatsChanged();
  return list;
}

export function deleteChat(endpoint: string, id: string): SavedChat[] {
  const list = loadChats(endpoint).filter((c) => c.id !== id);
  try {
    localStorage.setItem(keyFor(endpoint), JSON.stringify(list));
  } catch {
    /* best-effort */
  }
  notifyChatsChanged();
  return list;
}

function isSavedChat(v: unknown): v is SavedChat {
  if (v === null || typeof v !== "object") {
    return false;
  }
  const c = v as Record<string, unknown>;
  return (
    typeof c.id === "string" &&
    typeof c.title === "string" &&
    typeof c.updatedAt === "number" &&
    Array.isArray(c.messages)
  );
}
