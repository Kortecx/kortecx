import { m } from "framer-motion";
import { useState } from "react";
import { fadeUp } from "../../app/motion";
import { useAttachments } from "../../kx/use-attachments";
import { useChat } from "../../kx/use-chat";
import { type ChatSettings, loadChatSettings, saveChatSettings } from "../../lib/chat-settings";
import type { MessageAttachment } from "../../lib/chat-thread";
import { AttachmentStrip } from "./AttachmentStrip";
import { ChatSettingsPanel } from "./ChatSettings";
import { Composer } from "./Composer";
import { DegradeNotice } from "./DegradeNotice";
import { MessageList } from "./MessageList";
import { ModelPicker } from "./ModelPicker";
import { ThinkingTrace } from "./ThinkingTrace";

/**
 * The agentic chat. A message runs the configured recipe; the reply is the run's
 * committed result; the DAG-of-thought shows the run executing. Batch A: attach
 * images (uploaded via PutContent; they ride the vision recipe when the serve
 * is image-capable, display-only otherwise) and pick the model (a server-
 * validated free-param). Degrades to a guidance notice when no chat recipe /
 * model is provisioned.
 */
export function ChatPanel() {
  const [settings, setSettings] = useState<ChatSettings>(() => loadChatSettings());
  const chat = useChat({
    handle: settings.handle,
    promptKey: settings.promptKey,
    modelId: settings.modelId,
  });
  const attach = useAttachments();

  function updateSettings(next: ChatSettings): void {
    setSettings(next);
    saveChatSettings(next);
  }

  function sendWithAttachments(text: string): void {
    // Only READY uploads ride the message (failed/uploading chips stay behind
    // in the strip; the composer blocks sends while uploads are in flight).
    const ready: MessageAttachment[] = attach.attachments
      .filter((a) => a.status === "ready" && a.ref !== undefined)
      .map((a) => ({
        ref: a.ref as string,
        filename: a.filename,
        mediaType: a.mediaType,
        objectUrl: a.objectUrl,
      }));
    void chat.send(text, ready);
    attach.clear();
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
        <h1>New Chat</h1>
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
        onRetry={(id) => void chat.retry(id)}
        renderTrace={
          settings.showThinking
            ? (id) =>
                id === chat.activeAssistantId && chat.activeProjection ? (
                  <ThinkingTrace projection={chat.activeProjection} />
                ) : null
            : undefined
        }
      />

      <div className="composer__bar">
        <ModelPicker
          value={settings.modelId}
          onChange={(modelId) => updateSettings({ ...settings, modelId })}
        />
      </div>
      <AttachmentStrip attachments={attach.attachments} onRemove={attach.remove} />
      <Composer
        disabled={chat.busy}
        sendBlocked={attach.uploading}
        onSend={sendWithAttachments}
        onPickFiles={attach.addFiles}
      />
    </m.section>
  );
}
