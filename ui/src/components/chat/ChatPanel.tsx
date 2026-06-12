import { m } from "framer-motion";
import { useEffect, useRef, useState } from "react";
import { fadeUp } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { useAttachments } from "../../kx/use-attachments";
import { REACT_RECIPE_HANDLE, useChat } from "../../kx/use-chat";
import { useRecipes } from "../../kx/use-recipes";
import { type SavedChat, saveChat } from "../../lib/chat-history";
import { type ChatSettings, loadChatSettings, saveChatSettings } from "../../lib/chat-settings";
import type { MessageAttachment } from "../../lib/chat-thread";
import { Icon } from "../shell/Icon";
import { AttachmentStrip } from "./AttachmentStrip";
import { ChatHistory } from "./ChatHistory";
import { ChatSettingsPanel } from "./ChatSettings";
import { Composer } from "./Composer";
import { DegradeNotice } from "./DegradeNotice";
import { MessageList } from "./MessageList";
import { ModelPicker } from "./ModelPicker";
import { ReactProgress } from "./ReactProgress";
import { ThinkingTrace } from "./ThinkingTrace";

/**
 * The agentic chat. A message runs the configured recipe; the reply is the run's
 * committed result; the DAG-of-thought shows the run executing. Batch A: attach
 * images (uploaded via PutContent; they ride the vision recipe when the serve
 * is image-capable, display-only otherwise) and pick the model (a server-
 * validated free-param). PR-1.1: every thread autosaves to the client-local
 * per-endpoint history (restorable from the History slide-over) and the
 * composer is a Monaco markdown surface. Degrades to a guidance notice when no
 * chat recipe/model is provisioned.
 */
export function ChatPanel() {
  const { endpoint } = useConnection();
  const [settings, setSettings] = useState<ChatSettings>(() => loadChatSettings());
  const [agentMode, setAgentMode] = useState(false);
  // The agent toggle only EXISTS when the react loop is provisioned (an
  // inference serve with the bundled tool) — don't-fake-gaps.
  const recipes = useRecipes();
  const agentAvailable = (recipes.data ?? []).includes(REACT_RECIPE_HANDLE);
  const chat = useChat({
    handle: settings.handle,
    promptKey: settings.promptKey,
    modelId: settings.modelId,
    agentMode: agentMode && agentAvailable,
  });
  const attach = useAttachments();
  const [historyOpen, setHistoryOpen] = useState(false);
  // The identity the autosave upserts under; a new id per fresh/restored thread.
  const chatIdRef = useRef<string>(crypto.randomUUID());

  // Autosave: every thread change upserts this chat (empty threads are a no-op).
  useEffect(() => {
    saveChat(endpoint, chatIdRef.current, chat.thread.messages);
  }, [endpoint, chat.thread]);

  function updateSettings(next: ChatSettings): void {
    setSettings(next);
    saveChatSettings(next);
  }

  function newChat(): void {
    chatIdRef.current = crypto.randomUUID();
    chat.reset();
  }

  function loadSaved(saved: SavedChat): void {
    chatIdRef.current = saved.id;
    chat.loadThread(saved.messages);
    setHistoryOpen(false);
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
        <div className="screen__head-actions">
          <button
            type="button"
            className="iconbtn"
            onClick={() => setHistoryOpen(true)}
            aria-label="Chat history"
            title="Chat history"
            data-testid="chat-history-toggle"
          >
            <Icon name="history" />
          </button>
          {chat.thread.messages.length > 0 ? (
            <button type="button" className="linkbtn" onClick={newChat}>
              New chat
            </button>
          ) : null}
        </div>
      </div>
      <p className="muted">
        {agentMode && agentAvailable ? (
          <>
            Each message is a TASK for the agent loop (<code>{REACT_RECIPE_HANDLE}</code>): the
            model reasons and fires tools until it answers.
          </>
        ) : (
          <>
            Each message runs <code>{settings.handle}</code>; the reply is the run's committed
            result.
          </>
        )}
      </p>

      {agentAvailable ? (
        <fieldset className="view-toggle" aria-label="Chat mode" data-testid="chat-mode">
          <button
            type="button"
            aria-pressed={!agentMode}
            data-testid="chat-mode-chat"
            onClick={() => setAgentMode(false)}
          >
            Chat
          </button>
          <button
            type="button"
            aria-pressed={agentMode}
            data-testid="chat-mode-agent"
            onClick={() => setAgentMode(true)}
            title="Give the agent a task — it loops (reason → tool → observe) until it answers"
          >
            Agent task
          </button>
        </fieldset>
      ) : null}

      <ChatSettingsPanel settings={settings} onChange={updateSettings} />
      {chat.degraded ? <DegradeNotice error={chat.degraded} /> : null}

      <MessageList
        thread={chat.thread}
        autoscroll={settings.autoscroll}
        onRetry={(id) => void chat.retry(id)}
        renderTrace={(id) => {
          if (id !== chat.activeAssistantId) {
            return null;
          }
          return (
            <>
              {chat.reactTurns ? <ReactProgress turns={chat.reactTurns} /> : null}
              {settings.showThinking && chat.activeProjection ? (
                <ThinkingTrace projection={chat.activeProjection} />
              ) : null}
            </>
          );
        }}
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

      <ChatHistory
        endpoint={endpoint}
        open={historyOpen}
        onClose={() => setHistoryOpen(false)}
        onLoad={loadSaved}
      />
    </m.section>
  );
}
