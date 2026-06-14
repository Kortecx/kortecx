import { type ReactNode, useEffect, useRef } from "react";
import type { ChatThread } from "../../lib/chat-thread";
import { EmptyState } from "../EmptyState";
import { MessageBubble } from "./MessageBubble";

/** The scrollable thread; auto-scrolls to the newest message when enabled. */
export function MessageList({
  thread,
  autoscroll,
  showReasoning,
  renderTrace,
  onRetry,
  recipeHandle,
  modelId,
}: {
  thread: ChatThread;
  autoscroll: boolean;
  /** Show the model's `<think>` reasoning disclosure above the answer (T-FEAT1). */
  showReasoning: boolean;
  renderTrace?: (assistantId: string) => ReactNode;
  onRetry?: (assistantId: string) => void;
  /** The chat backing handle/model — advisory context on 👍/👎 feedback (PR-4.1). */
  recipeHandle?: string;
  modelId?: string;
}) {
  const endRef = useRef<HTMLDivElement>(null);

  // biome-ignore lint/correctness/useExhaustiveDependencies: scroll on any thread change.
  useEffect(() => {
    if (!autoscroll) {
      return;
    }
    try {
      endRef.current?.scrollIntoView({ block: "end" });
    } catch {
      /* jsdom has no layout — harmless */
    }
  }, [thread, autoscroll]);

  if (thread.messages.length === 0) {
    return <EmptyState title="No messages yet" detail="Ask the runtime something below." />;
  }

  return (
    <div className="chat__list" data-testid="message-list">
      {thread.messages.map((m) => (
        <MessageBubble
          key={m.id}
          message={m}
          showReasoning={showReasoning}
          trace={m.role === "assistant" ? renderTrace?.(m.id) : undefined}
          onRetry={m.role === "assistant" ? onRetry : undefined}
          recipeHandle={recipeHandle}
          modelId={modelId}
        />
      ))}
      <div ref={endRef} />
    </div>
  );
}
