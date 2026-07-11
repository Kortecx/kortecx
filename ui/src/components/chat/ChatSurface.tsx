/**
 * POC-5d: the chat PRESENTATION surface, extracted verbatim from `ChatPanel` so
 * the standalone New Chat route and the embedded `AppChat` share one body with no
 * drift. Driven by a {@link ChatController} (the orchestration hook) plus flags:
 *   - showModeToggle — the Chat / Agent-task fieldset (standalone only)
 *   - showPickers    — the model + dataset composer controls + attach (standalone)
 *   - showHistory    — the Export / History / New-chat head actions + slide-over
 *   - header         — a caller-supplied head block (AppChat); else the standalone
 *                      editable chat-name + actions head.
 *
 * EVERY existing chat testid lands on the SAME DOM node as before (chat-panel is
 * the section, chat-name/chat-export/chat-history-toggle/chat-new in the head,
 * chat-mode-*, chat-context-*, composer/message testids) — the standalone path is
 * byte-identical (the regression gate).
 */

import { m } from "framer-motion";
import { useState } from "react";
import { fadeUp } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { CHAT_RAG_RECIPE_HANDLE, REACT_RECIPE_HANDLE } from "../../kx/use-chat";
import { Icon } from "../shell/Icon";
import { AttachmentStrip } from "./AttachmentStrip";
import { ChatHistory } from "./ChatHistory";
import { ChatSettingsPanel } from "./ChatSettings";
import { Composer } from "./Composer";
import { DatasetPicker } from "./DatasetPicker";
import { DegradeNotice } from "./DegradeNotice";
import { McpConnectionChips } from "./McpConnectionChips";
import { MessageList } from "./MessageList";
import { ModelPicker } from "./ModelPicker";
import { StatusLoop } from "./StatusLoop";
import { ThinkingTrace } from "./ThinkingTrace";
import type { ChatController } from "./useChatController";

export interface ChatSurfaceProps {
  controller: ChatController;
  /** Show the model + dataset composer controls (+ attach menu). */
  showPickers?: boolean;
  /** Show the Export / History / New-chat head actions + the history slide-over. */
  showHistory?: boolean;
  /** Show the Chat / Agent-task mode fieldset. */
  showModeToggle?: boolean;
  /** A caller-supplied head; when omitted the standalone editable name head shows. */
  header?: React.ReactNode;
  /** The section testid (the standalone keeps `chat-panel`; AppChat uses `app-chat`). */
  sectionTestId?: string;
}

export function ChatSurface({
  controller,
  showPickers = true,
  showHistory = true,
  showModeToggle = true,
  header,
  sectionTestId = "chat-panel",
}: ChatSurfaceProps) {
  const { endpoint } = useConnection();
  const [historyOpen, setHistoryOpen] = useState(false);
  const {
    chat,
    settings,
    updateSettings,
    agentMode,
    setAgentMode,
    agentAvailable,
    agentTurn,
    dataset,
    setDataset,
    backingHandle,
    promptNoModel,
    attach,
    contextBundles,
    pendingContext,
    toggleContext,
    pendingTools,
    toggleTool,
    toolRegistry,
    mcpServers,
    chatName,
    onChatNameInput,
    commitName,
    newChat,
    loadSaved,
    exportChat,
    sendWithAttachments,
  } = controller;

  return (
    <m.section
      className="screen chat"
      data-testid={sectionTestId}
      variants={fadeUp}
      initial="hidden"
      animate="show"
    >
      {header !== undefined ? (
        header
      ) : (
        <div className="screen__head chat__head">
          <div className="chat__title-block">
            <input
              className="chat__name-input"
              value={chatName}
              onChange={(e) => onChatNameInput(e.target.value)}
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
          {showHistory ? (
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
          ) : null}
        </div>
      )}
      <p className="muted">
        {agentTurn ? (
          <>
            Each message is a TASK for the agent loop (<code>{REACT_RECIPE_HANDLE}</code>): the
            model reasons and fires tools until it answers.
          </>
        ) : dataset ? (
          <>
            Each message runs <code>{CHAT_RAG_RECIPE_HANDLE}</code>, grounded on dataset{" "}
            <strong data-testid="chat-grounded-on">{dataset}</strong> — the retrieved documents fold
            into the prompt before the model answers (plain chat if the dataset is empty).
          </>
        ) : (
          <>
            Each message runs <code>{backingHandle}</code>; the reply is the run's committed result.
          </>
        )}
      </p>

      {showModeToggle && agentAvailable ? (
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

      {showPickers ? <ChatSettingsPanel settings={settings} onChange={updateSettings} /> : null}
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
        recipeHandle={backingHandle}
        modelId={settings.modelId}
        renderTrace={(id) => {
          if (id !== chat.activeAssistantId) {
            return null;
          }
          return (
            <>
              <StatusLoop chat={chat} />
              {settings.showThinking && chat.activeProjection ? (
                <ThinkingTrace projection={chat.activeProjection} />
              ) : null}
            </>
          );
        }}
      />

      {showPickers ? (
        <div className="composer__bar">
          <ModelPicker
            value={settings.modelId}
            onChange={(modelId) => updateSettings({ ...settings, modelId })}
          />
          {agentTurn ? null : <DatasetPicker value={dataset} onChange={setDataset} />}
        </div>
      ) : null}
      {showPickers ? (
        <AttachmentStrip attachments={attach.attachments} onRemove={attach.remove} />
      ) : null}
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
      {showPickers ? <McpConnectionChips servers={mcpServers.servers} /> : null}
      <Composer
        disabled={chat.busy}
        sendBlocked={attach.uploading}
        onSend={sendWithAttachments}
        onPickFiles={showPickers ? attach.addFiles : undefined}
        context={
          showPickers
            ? {
                bundles: contextBundles.bundles.map((b) => b.handle),
                attached: pendingContext,
                notWired: contextBundles.notWired,
                onToggle: toggleContext,
              }
            : undefined
        }
        tools={
          showPickers
            ? {
                options: toolRegistry.tools
                  .filter((t) => t.registrationStatus === "Approved")
                  .map((t) => `${t.toolName}@${t.toolVersion}`),
                attached: pendingTools,
                notWired: toolRegistry.notWired,
                onToggle: toggleTool,
              }
            : undefined
        }
      />

      {showHistory ? (
        <ChatHistory
          endpoint={endpoint}
          open={historyOpen}
          onClose={() => setHistoryOpen(false)}
          onLoad={(saved) => {
            loadSaved(saved);
            setHistoryOpen(false);
          }}
        />
      ) : null}
    </m.section>
  );
}
