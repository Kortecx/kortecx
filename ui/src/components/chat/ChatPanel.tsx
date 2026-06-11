import { m } from "framer-motion";
import { useState } from "react";
import { fadeUp } from "../../app/motion";
import { useChat } from "../../kx/use-chat";
import { type ChatSettings, loadChatSettings, saveChatSettings } from "../../lib/chat-settings";
import { ChatSettingsPanel } from "./ChatSettings";
import { Composer } from "./Composer";
import { DegradeNotice } from "./DegradeNotice";
import { MessageList } from "./MessageList";
import { ThinkingTrace } from "./ThinkingTrace";

/**
 * The agentic chat. A message runs the configured recipe; the reply is the run's
 * committed result; the DAG-of-thought shows the run executing. Degrades to a
 * guidance notice when no chat recipe/model is provisioned.
 */
export function ChatPanel() {
  const [settings, setSettings] = useState<ChatSettings>(() => loadChatSettings());
  const chat = useChat({ handle: settings.handle, promptKey: settings.promptKey });

  function updateSettings(next: ChatSettings): void {
    setSettings(next);
    saveChatSettings(next);
  }

  return (
    <m.section
      className="screen chat"
      data-testid="chat-panel"
      variants={fadeUp}
      initial="hidden"
      animate="show"
    >
      <div className="screen__head">
        <h1>Chat</h1>
        {chat.thread.messages.length > 0 ? (
          <button type="button" className="linkbtn" onClick={chat.reset}>
            New chat
          </button>
        ) : null}
      </div>
      <p className="muted">
        Each message runs <code>{settings.handle}</code>; the reply is the run's committed result.
      </p>

      <ChatSettingsPanel settings={settings} onChange={updateSettings} />
      {chat.degraded ? <DegradeNotice error={chat.degraded} /> : null}

      <MessageList
        thread={chat.thread}
        autoscroll={settings.autoscroll}
        renderTrace={
          settings.showThinking
            ? (id) =>
                id === chat.activeAssistantId && chat.activeProjection ? (
                  <ThinkingTrace projection={chat.activeProjection} />
                ) : null
            : undefined
        }
      />

      <Composer disabled={chat.busy} onSend={(t) => void chat.send(t)} />
    </m.section>
  );
}
