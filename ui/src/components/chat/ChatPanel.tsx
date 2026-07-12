/**
 * The READ-ONLY, RAG-grounded New Chat (standalone route). A message retrieves over
 * the user's own DATASETS + CONTEXT FILES (picked in the {@link GroundingBar}) and
 * the model answers grounded on the retrieved documents (chat-rag); a settled
 * grounded answer shows its source citations. Read-only: it retrieves + reasons over
 * stored context, never mutating it — the mutate-capable agentic chat (agent task +
 * tools) lives in App chat — the capability is relocated, not crippled.
 * Pick the model, attach an image (the vision route, form-gated), and every thread
 * autosaves to the client-local per-endpoint history. Degrades to a guidance notice
 * when no chat recipe/model is provisioned.
 *
 * POC-5d: the orchestration lives in {@link useChatController} and the body in
 * {@link ChatSurface} (shared with the embedded {@link AppChat}); this panel is a
 * thin wrapper. The frozen `chat-panel` section id + the head/composer testids are
 * unchanged (the regression gate).
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
      showGrounding
      sectionTestId="chat-panel"
    />
  );
}
