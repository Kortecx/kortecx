/**
 * The agentic chat (standalone New Chat route). A message runs the configured
 * recipe; the reply is the run's committed result; the DAG-of-thought shows the
 * run executing. Attach images (the vision route, form-gated), pick the model,
 * ground over a dataset (chat-rag), give the agent a task (the react loop), or
 * attach context bundles. Every thread autosaves to the client-local per-endpoint
 * history. Degrades to a guidance notice when no chat recipe/model is provisioned.
 *
 * POC-5d: the orchestration moved into {@link useChatController} and the body into
 * {@link ChatSurface} (shared with the embedded {@link AppChat}); this panel is a
 * thin wrapper that drives the full interactive surface. The DOM + every testid
 * are byte-identical to the pre-refactor panel (the regression gate).
 */

import { ChatSurface } from "./ChatSurface";
import { useChatController } from "./useChatController";

export function ChatPanel() {
  const controller = useChatController();
  return (
    <ChatSurface
      controller={controller}
      showPickers
      showHistory
      showModeToggle
      sectionTestId="chat-panel"
    />
  );
}
