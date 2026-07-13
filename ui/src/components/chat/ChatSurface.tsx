/**
 * POC-5d: the chat PRESENTATION surface, extracted verbatim from `ChatPanel` so
 * the standalone New Chat route and the embedded `AppChat` share one body with no
 * drift. Driven by a {@link ChatController} (the orchestration hook) plus flags:
 *   - showPickers    — the model composer control + file attach (standalone)
 *   - showHistory    — the Export / History / New-chat head actions + slide-over
 *   - showGrounding  — the read-only RAG grounding bar (dataset + context files)
 *   - header         — a caller-supplied head block (AppChat); else the standalone
 *                      editable chat-name + actions head.
 *
 * PR-A: the standalone New Chat is READ-ONLY, RAG-grounded — no Agent
 * toggle, no tool picker, no MCP chips; dataset + context selection is the headline
 * {@link GroundingBar}, and a settled grounded answer renders its {@link
 * MessageSources}. The mutate-capable agentic chat lives in App chat (unchanged).
 */

import { m } from "framer-motion";
import { useState } from "react";
import { fadeUp } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { CHAT_RAG_RECIPE_HANDLE, REACT_RECIPE_HANDLE } from "../../kx/use-chat";
import type { ChatMessage } from "../../lib/chat-thread";
import { Icon } from "../shell/Icon";
import { AttachmentStrip } from "./AttachmentStrip";
import { ChatHistory } from "./ChatHistory";
import { ChatSettingsPanel } from "./ChatSettings";
import { Composer } from "./Composer";
import { ContextAttachButton } from "./ContextAttachButton";
import { DatasetPicker } from "./DatasetPicker";
import { DegradeNotice } from "./DegradeNotice";
import { GroundingBar } from "./GroundingBar";
import { MessageList } from "./MessageList";
import { MessageSources } from "./MessageSources";
import { ModelPicker } from "./ModelPicker";
import { StatusLoop } from "./StatusLoop";
import { ThinkingTrace } from "./ThinkingTrace";
import type { ChatController } from "./useChatController";

export interface ChatSurfaceProps {
  controller: ChatController;
  /** Show the model composer control (+ the file attach menu). */
  showPickers?: boolean;
  /** Show the Export / History / New-chat head actions + the history slide-over. */
  showHistory?: boolean;
  /** Show the read-only RAG grounding bar (dataset + context files). */
  showGrounding?: boolean;
  /** Show the in-flight run graph (the "DAG-of-thought"). Off for the read-only New
   *  Chat — graphs live on the run-review surface, never in the chat window. */
  showThinkingGraph?: boolean;
  /** A caller-supplied head; when omitted the standalone editable name head shows. */
  header?: React.ReactNode;
  /** The section testid (the standalone keeps `chat-panel`; AppChat uses `app-chat`). */
  sectionTestId?: string;
}

export function ChatSurface({
  controller,
  showPickers = true,
  showHistory = true,
  showGrounding = false,
  showThinkingGraph = true,
  header,
  sectionTestId = "chat-panel",
}: ChatSurfaceProps) {
  const { endpoint } = useConnection();
  const [historyOpen, setHistoryOpen] = useState(false);
  const {
    chat,
    settings,
    updateSettings,
    agentTurn,
    dataset,
    setDataset,
    backingHandle,
    promptNoModel,
    attach,
    contextBundles,
    pendingContext,
    toggleContext,
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
                className="btn-primary chat__new"
                onClick={newChat}
                disabled={chat.thread.messages.length === 0}
                title="Start a new chat"
                data-testid="chat-new"
              >
                <Icon name="plus" />
                <span>New chat</span>
              </button>
              {showGrounding ? (
                <ContextAttachButton
                  bundles={contextBundles.bundles.map((b) => b.handle)}
                  attached={pendingContext}
                  notWired={contextBundles.notWired}
                  onToggle={toggleContext}
                />
              ) : null}
              <button
                type="button"
                className="iconbtn"
                onClick={exportChat}
                disabled={chat.thread.messages.length === 0}
                title="Export this chat as JSON"
                aria-label="Export chat"
                data-testid="chat-export"
              >
                <Icon name="download" />
              </button>
              <button
                type="button"
                className="iconbtn"
                onClick={() => setHistoryOpen(true)}
                title="Chat history"
                aria-label="Chat history"
                data-testid="chat-history-toggle"
              >
                <Icon name="history" />
              </button>
            </div>
          ) : null}
        </div>
      )}
      {/* The recipe-mechanics note is dev context — hidden on the read-only New Chat
          (grounding + the answer speak for themselves); kept for the embedded App chat. */}
      {!showGrounding ? (
        <p className="muted">
          {agentTurn ? (
            <>
              Each message is a TASK for the agent loop (<code>{REACT_RECIPE_HANDLE}</code>): the
              model reasons and fires tools until it answers.
            </>
          ) : dataset ? (
            <>
              Each message runs <code>{CHAT_RAG_RECIPE_HANDLE}</code>, grounded on dataset{" "}
              <strong>{dataset}</strong> — the retrieved documents fold into the prompt before the
              model answers (plain chat if the dataset is empty).
            </>
          ) : (
            <>
              Each message runs <code>{backingHandle}</code>; the reply is the run's committed
              result.
            </>
          )}
        </p>
      ) : null}

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
        renderSources={
          showGrounding
            ? (msg: ChatMessage) => (
                <MessageSources
                  instanceId={msg.instanceId}
                  moteId={msg.terminalMoteId}
                  active={msg.status === "done"}
                />
              )
            : undefined
        }
        renderTrace={(id) => {
          if (id !== chat.activeAssistantId) {
            return null;
          }
          return (
            <>
              <StatusLoop chat={chat} />
              {showThinkingGraph && settings.showThinking && chat.activeProjection ? (
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
          {/* The dataset control lives in the grounding bar when grounding is on
              (the read-only chat); otherwise it sits inline (unless this is an
              agent turn, which grounds via the react-rag `retrieve` tool). */}
          {agentTurn || showGrounding ? null : (
            <DatasetPicker value={dataset} onChange={setDataset} />
          )}
        </div>
      ) : null}
      {/* Grounding (dataset + status + attached-context chips) sits above the input;
          the Context ATTACH control lives in the header (ContextAttachButton). */}
      {showGrounding ? (
        <GroundingBar
          dataset={dataset}
          onDataset={setDataset}
          attached={pendingContext}
          onToggleContext={toggleContext}
        />
      ) : null}
      {/* Model + display settings sit JUST ABOVE the input (moved off the top). */}
      {showPickers ? <ChatSettingsPanel settings={settings} onChange={updateSettings} /> : null}
      {showPickers ? (
        <AttachmentStrip attachments={attach.attachments} onRemove={attach.remove} />
      ) : null}
      <Composer
        disabled={chat.busy}
        sendBlocked={attach.uploading}
        busy={chat.busy}
        onSend={sendWithAttachments}
        onStop={() => chat.cancel()}
        onPickFiles={showPickers ? attach.addFiles : undefined}
        context={
          // Context selection moves to the grounding bar when grounding is on;
          // otherwise it stays in the composer attach menu.
          showPickers && !showGrounding
            ? {
                bundles: contextBundles.bundles.map((b) => b.handle),
                attached: pendingContext,
                notWired: contextBundles.notWired,
                onToggle: toggleContext,
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
