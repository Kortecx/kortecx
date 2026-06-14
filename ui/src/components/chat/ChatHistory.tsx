import { useEffect, useState } from "react";
import { CHATS_CHANGED_EVENT, type SavedChat, deleteChat, loadChats } from "../../lib/chat-history";
import { EmptyState } from "../EmptyState";

/**
 * The chat-history slide-over: every saved chat on THIS endpoint (client-local
 * localStorage — the open-question-5 decision; server-side sessions are a
 * future sidecar), newest-updated first. Click restores a thread; the × forgets
 * it. Follows the activity-drawer slide-over interaction (backdrop + Escape).
 */
export function ChatHistory({
  endpoint,
  open,
  onClose,
  onLoad,
}: {
  endpoint: string;
  open: boolean;
  onClose: () => void;
  onLoad: (chat: SavedChat) => void;
}) {
  const [chats, setChats] = useState<SavedChat[]>([]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const refresh = (): void => setChats(loadChats(endpoint));
    refresh();
    window.addEventListener(CHATS_CHANGED_EVENT, refresh);
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener(CHATS_CHANGED_EVENT, refresh);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, endpoint, onClose]);

  if (!open) {
    return null;
  }

  return (
    <>
      <button
        type="button"
        className="chat-history__backdrop"
        aria-label="Close chat history"
        onClick={onClose}
        data-testid="chat-history-close"
      />
      <aside className="chat-history" data-testid="chat-history" aria-label="Chat history">
        <div className="chat-history__head">
          <h2>Chat history</h2>
          <button type="button" className="linkbtn" onClick={onClose}>
            Close
          </button>
        </div>
        <p className="muted chat-history__note">
          Saved in this browser for <code>{endpoint}</code>.
        </p>
        {chats.length === 0 ? (
          <EmptyState title="No saved chats" detail="Finished chats appear here automatically." />
        ) : (
          <ul className="chat-history__list">
            {chats.map((c) => (
              <li key={c.id} className="chat-history__item" data-testid="chat-history-item">
                <button
                  type="button"
                  className="chat-history__load"
                  onClick={() => onLoad(c)}
                  data-testid="chat-history-load"
                >
                  <span className="chat-history__title">{c.name ?? c.title}</span>
                  <span className="muted">
                    {c.title} · {new Date(c.updatedAt).toLocaleString()} · {c.messages.length}{" "}
                    message
                    {c.messages.length === 1 ? "" : "s"}
                  </span>
                </button>
                <button
                  type="button"
                  className="iconbtn chat-history__delete"
                  onClick={() => setChats(deleteChat(endpoint, c.id))}
                  aria-label={`Delete chat: ${c.name ?? c.title}`}
                  data-testid="chat-history-delete"
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
        )}
      </aside>
    </>
  );
}
