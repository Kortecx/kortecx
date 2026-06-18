import { m } from "framer-motion";
import { useEffect, useRef, useState } from "react";
import { fadeUp } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { useAttachments } from "../../kx/use-attachments";
import { REACT_RECIPE_HANDLE, useChat } from "../../kx/use-chat";
import { useContextBundles } from "../../kx/use-context-bundles";
import { useModels } from "../../kx/use-models";
import { useRecipes } from "../../kx/use-recipes";
import {
  type SavedChat,
  autoNameFrom,
  defaultChatName,
  renameChat,
  saveChat,
} from "../../lib/chat-history";
import {
  type ChatSettings,
  loadChatSettings,
  resolveChatBacking,
  saveChatSettings,
  shouldPromptNoModel,
} from "../../lib/chat-settings";
import type { MessageAttachment } from "../../lib/chat-thread";
import { download } from "../../lib/download";
import { exportChatFilename, exportChatJson } from "../../lib/export-chat";
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
  const available = recipes.data ?? [];
  const agentAvailable = available.includes(REACT_RECIPE_HANDLE);
  // Reconcile the persisted chat handle against the serve's LIVE recipes so a
  // stale model-free `echo` handle can't silently echo the prompt when a model is
  // provisioned (GR15). The model chat recipe backs chat whenever served.
  const backing = resolveChatBacking(settings, available);
  // Proactively surface the honest "no model — connect one" state on a no-model
  // serve (GR15 §2.208 backlog), BEFORE a send silently echoes. Gated on the
  // backing NOT being a deliberate `echo` choice — that path is honored verbatim
  // (resolveChatBacking's contract; the echo e2e + Settings preset stay green).
  const models = useModels();
  const promptNoModel = shouldPromptNoModel({
    modelCount: models.models?.length,
    loading: models.loading,
    unsupported: models.unsupported,
    backingHandle: backing.handle,
  });
  const chat = useChat({
    handle: backing.handle,
    promptKey: backing.promptKey,
    modelId: settings.modelId,
    agentMode: agentMode && agentAvailable,
  });
  const attach = useAttachments();
  // PR-7b: the party's authored context bundles + the handles attached to the
  // NEXT turn (multi-select via the composer attach-menu Context category).
  const contextBundles = useContextBundles();
  const [pendingContext, setPendingContext] = useState<readonly string[]>([]);
  function toggleContext(handle: string): void {
    setPendingContext((prev) =>
      prev.includes(handle) ? prev.filter((h) => h !== handle) : [...prev, handle],
    );
  }
  const [historyOpen, setHistoryOpen] = useState(false);
  // The identity the autosave upserts under; a new id per fresh/restored thread.
  const chatIdRef = useRef<string>(crypto.randomUUID());
  // The editable chat name (defaults to the creation timestamp). A ref mirrors it
  // so the thread-keyed autosave reads the latest name without re-subscribing on
  // every keystroke.
  const [chatName, setChatName] = useState<string>(() => defaultChatName());
  const chatNameRef = useRef(chatName);
  chatNameRef.current = chatName;
  // True once the user edits the name OR restores a named chat — auto-naming then
  // never overrides their choice.
  const userRenamedRef = useRef(false);

  // Autosave: every thread change upserts this chat (empty threads are a no-op),
  // carrying the current name.
  useEffect(() => {
    saveChat(endpoint, chatIdRef.current, chat.thread.messages, chatNameRef.current);
  }, [endpoint, chat.thread]);

  function updateSettings(next: ChatSettings): void {
    setSettings(next);
    saveChatSettings(next);
  }

  function newChat(): void {
    chatIdRef.current = crypto.randomUUID();
    setChatName(defaultChatName());
    userRenamedRef.current = false;
    chat.reset();
  }

  function loadSaved(saved: SavedChat): void {
    chatIdRef.current = saved.id;
    setChatName(saved.name ?? saved.title);
    // A restored chat already carries a name — never auto-rename it.
    userRenamedRef.current = true;
    chat.loadThread(saved.messages);
    setHistoryOpen(false);
  }

  function exportChat(): void {
    if (chat.thread.messages.length === 0) {
      return;
    }
    download(
      exportChatFilename(chatName),
      exportChatJson(chatName, chat.thread.messages),
      "application/json",
    );
  }

  // Persist a name edit — only once the chat actually exists in history.
  function commitName(): void {
    if (chat.thread.messages.length > 0) {
      renameChat(endpoint, chatIdRef.current, chatName);
    }
  }

  function sendWithAttachments(text: string): void {
    // Auto-name a fresh, un-renamed thread from its first message — set the ref
    // synchronously so THIS tick's autosave persists the derived name.
    if (!userRenamedRef.current && chat.thread.messages.length === 0) {
      const auto = autoNameFrom([{ id: "seed", role: "user", text, status: "done" }]);
      if (auto) {
        setChatName(auto);
        chatNameRef.current = auto;
      }
    }
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
    void chat.send(text, ready, pendingContext);
    attach.clear();
    setPendingContext([]);
  }

  return (
    <m.section
      className="screen chat"
      data-testid="chat-panel"
      variants={fadeUp}
      initial="hidden"
      animate="show"
    >
      <div className="screen__head chat__head">
        <div className="chat__title-block">
          <input
            className="chat__name-input"
            value={chatName}
            onChange={(e) => {
              userRenamedRef.current = true;
              setChatName(e.target.value);
            }}
            onBlur={commitName}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                e.currentTarget.blur();
              }
            }}
            aria-label="Chat name"
            data-testid="chat-name"
            spellCheck={false}
            autoComplete="off"
          />
        </div>
        <div className="screen__head-actions chat__head-actions">
          <button
            type="button"
            className="btn-ghost"
            onClick={exportChat}
            disabled={chat.thread.messages.length === 0}
            title="Export this chat as JSON"
            data-testid="chat-export"
          >
            <Icon name="download" />
            <span>Export</span>
          </button>
          <button
            type="button"
            className="btn-ghost"
            onClick={() => setHistoryOpen(true)}
            title="Chat history"
            data-testid="chat-history-toggle"
          >
            <Icon name="history" />
            <span>History</span>
          </button>
          <button
            type="button"
            className="btn-ghost"
            onClick={newChat}
            disabled={chat.thread.messages.length === 0}
            title="Start a new chat"
            data-testid="chat-new"
          >
            <Icon name="plus" />
            <span>New chat</span>
          </button>
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
            Each message runs <code>{backing.handle}</code>; the reply is the run's committed
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
      {chat.degraded ? (
        <DegradeNotice error={chat.degraded} />
      ) : promptNoModel ? (
        <DegradeNotice />
      ) : null}

      <MessageList
        thread={chat.thread}
        autoscroll={settings.autoscroll}
        showReasoning={settings.showReasoning}
        onRetry={(id) => void chat.retry(id)}
        recipeHandle={backing.handle}
        modelId={settings.modelId}
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
      {pendingContext.length > 0 ? (
        <div className="context-strip" data-testid="chat-context-strip">
          <span className="context-strip__label muted">Context:</span>
          {pendingContext.map((handle) => (
            <span
              key={handle}
              className="context-strip__chip"
              data-testid={`chat-context-${handle}`}
            >
              <span className="mono">{handle}</span>
              <button
                type="button"
                className="context-strip__remove"
                aria-label={`Detach ${handle}`}
                data-testid={`chat-context-remove-${handle}`}
                onClick={() => toggleContext(handle)}
              >
                ✕
              </button>
            </span>
          ))}
        </div>
      ) : null}
      <Composer
        disabled={chat.busy}
        sendBlocked={attach.uploading}
        onSend={sendWithAttachments}
        onPickFiles={attach.addFiles}
        context={{
          bundles: contextBundles.bundles.map((b) => b.handle),
          attached: pendingContext,
          notWired: contextBundles.notWired,
          onToggle: toggleContext,
        }}
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
