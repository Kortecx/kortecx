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
  /** User-editable display name; defaults to the creation timestamp. OPTIONAL so
   *  records written before this field (no `name`) still load — readers fall back
   *  to `title`. */
  readonly name?: string;
  /** Derived first-message preview (kept as a subtitle + legacy fallback). */
  readonly title: string;
  readonly createdAt: number;
  readonly updatedAt: number;
  readonly messages: readonly ChatMessage[];
}

/** The default name for a fresh chat: a sortable, human local timestamp. */
export function defaultChatName(now: number = Date.now()): string {
  return new Date(now).toLocaleString();
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

/** A short auto-generated name derived from a thread's first user message (the
 *  `chatTitle` shape, capped tighter for a name-line). `null` when there is no
 *  user message yet to name from — the caller keeps the timestamp default. */
const AUTO_NAME_MAX = 48;
export function autoNameFrom(messages: readonly ChatMessage[]): string | null {
  const first = messages
    .find((m) => m.role === "user")
    ?.text?.replace(/\s+/g, " ")
    .trim();
  if (!first) {
    return null;
  }
  return first.length > AUTO_NAME_MAX ? `${first.slice(0, AUTO_NAME_MAX)}…` : first;
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
  name?: string,
  now: number = Date.now(),
): SavedChat[] {
  if (messages.length === 0) {
    return loadChats(endpoint);
  }
  const existing = loadChats(endpoint);
  const prior = existing.find((c) => c.id === id);
  // The name is the user's (a timestamp by default) — preserved across autosaves,
  // never recomputed from the messages (that is `title`'s job).
  const resolvedName = name?.trim() || prior?.name || defaultChatName(prior?.createdAt ?? now);
  const next: SavedChat = {
    id,
    name: resolvedName,
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

/** Rename a saved chat by id (no-op if absent, or the new name is blank). */
export function renameChat(endpoint: string, id: string, name: string): SavedChat[] {
  const trimmed = name.trim();
  if (trimmed === "") {
    return loadChats(endpoint);
  }
  const list = loadChats(endpoint).map((c) => (c.id === id ? { ...c, name: trimmed } : c));
  try {
    localStorage.setItem(keyFor(endpoint), JSON.stringify(list));
  } catch {
    /* best-effort */
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
