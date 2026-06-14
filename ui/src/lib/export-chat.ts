/**
 * Serialize a chat thread to a stable, human-readable JSON document (the
 * "Export chat" affordance). A pure transform over the persisted {@link SavedChat}
 * shape: every message's role/text/status plus the run attribution it carries,
 * and a small envelope (name, timestamps, version) so an exported file is
 * self-describing. Transient `blob:` previews are never serialized — only the
 * server-derived content refs that survive a reload.
 */

import type { SavedChat } from "./chat-history";
import type { ChatMessage } from "./chat-thread";

/** The on-disk export version (bump on a shape change). */
const EXPORT_VERSION = 1;

interface ExportMessage {
  readonly id: string;
  readonly role: ChatMessage["role"];
  readonly text: string;
  readonly status: ChatMessage["status"];
  readonly instanceId?: string;
  readonly terminalMoteId?: string;
  readonly attachments?: ReadonlyArray<{ ref: string; filename: string; mediaType: string }>;
}

/** A safe, slugged filename for an exported chat (never empty, no path chars). */
export function exportChatFilename(name: string, now: number = Date.now()): string {
  const slug =
    name
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 48) || "chat";
  return `kortecx-chat-${slug}-${now}.json`;
}

/** Serialize a chat (name + thread) to a stable JSON string. */
export function exportChatJson(
  name: string,
  messages: readonly ChatMessage[],
  meta: { createdAt?: number; updatedAt?: number } = {},
): string {
  const out = {
    kind: "kortecx.chat",
    version: EXPORT_VERSION,
    name,
    created_at: meta.createdAt,
    updated_at: meta.updatedAt,
    messages: messages.map(toExportMessage),
  };
  return JSON.stringify(out, null, 2);
}

/** Serialize a {@link SavedChat} (history entry) — the same envelope as the live thread. */
export function exportSavedChatJson(saved: SavedChat): string {
  return exportChatJson(saved.name ?? saved.title, saved.messages, {
    createdAt: saved.createdAt,
    updatedAt: saved.updatedAt,
  });
}

function toExportMessage(m: ChatMessage): ExportMessage {
  return {
    id: m.id,
    role: m.role,
    text: m.text,
    status: m.status,
    instanceId: m.instanceId,
    terminalMoteId: m.terminalMoteId,
    attachments: m.attachments?.map((a) => ({
      ref: a.ref,
      filename: a.filename,
      mediaType: a.mediaType,
    })),
  };
}
