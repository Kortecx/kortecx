/**
 * POC-5d: the embedded App chat — a conversation scoped to one App, reusing the
 * shared {@link ChatSurface} body + {@link useChatController} orchestration with
 * the interactive chrome OFF (no model/dataset pickers, no history slide-over, no
 * Chat/Agent toggle, no autosave). The App fixes the recipe (its agent task loop
 * when requested) + the model + the context refs; a user just types and sends.
 *
 * The App's project context (`contextRefs`) attaches to EVERY turn so the chat is
 * grounded on the App without the user re-attaching it. Authority is unchanged:
 * the server re-resolves every warrant at run (SN-8).
 */

import { MODEL_CHAT_HANDLE } from "../../lib/chat-settings";
import { ChatSurface } from "./ChatSurface";
import { useChatController } from "./useChatController";

export function AppChat({
  recipeHandle,
  modelId,
  contextRefs,
  agentMode,
}: {
  /** The App's recipe/handle. Display + grounding; the chat itself runs the
   *  model chat recipe (or the agent loop when `agentMode`), grounded on the
   *  App's `contextRefs`. */
  recipeHandle?: string;
  modelId?: string;
  contextRefs?: readonly string[];
  agentMode?: boolean;
}) {
  const controller = useChatController({
    // The App pins chat to the model chat recipe (the prompt binds to `prompt`),
    // or forces the agent loop when the App is an agentic App.
    backing: { handle: MODEL_CHAT_HANDLE, promptKey: "prompt" },
    modelId,
    agentMode: agentMode ?? false,
    autosave: false,
    contextRefs,
  });

  return (
    <ChatSurface
      controller={controller}
      showPickers={false}
      showHistory={false}
      showModeToggle={false}
      header={
        <div className="screen__head chat__head" data-testid="app-chat-head">
          <span className="muted">
            Chat scoped to <code className="mono">{recipeHandle ?? "this App"}</code> — grounded on
            its context.
          </span>
        </div>
      }
      sectionTestId="app-chat"
    />
  );
}
