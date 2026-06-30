/**
 * POC-5d: the chat ORCHESTRATION hook, extracted verbatim from `ChatPanel` so the
 * same agentic-chat machinery backs both the standalone New Chat route AND the
 * embedded App chat (`AppChat`) — no behavioural drift (the regression gate). The
 * presentational body lives in {@link ChatSurface}; this hook owns all the I/O +
 * derived state.
 *
 * Config-driven: pass `backing` to PIN the recipe (AppChat) or omit it to drive
 * chat from the user's persisted ChatSettings (the standalone route). `agentMode`
 * / `dataset` are likewise overridable-or-interactive. `autosave` gates the
 * client-local history upsert (off for an embedded App chat). `contextRefs` are
 * App-fixed bundle handles attached to every turn (additive over the per-message
 * context). Everything ChatPanel previously computed inline is returned here, so
 * ChatPanel becomes a thin wrapper that renders `<ChatSurface>` with the result.
 */

import { useEffect, useRef, useState } from "react";
import { useConnection } from "../../kx/connection-context";
import { useAttachments } from "../../kx/use-attachments";
import { REACT_RECIPE_HANDLE, type UseChat, useChat } from "../../kx/use-chat";
import { useContextBundles } from "../../kx/use-context-bundles";
import { useDefaultModel } from "../../kx/use-default-model";
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
import type { ChatMessage, MessageAttachment } from "../../lib/chat-thread";
import { download } from "../../lib/download";
import { exportChatFilename, exportChatJson } from "../../lib/export-chat";

export interface ChatControllerConfig {
  /** Pin the backing recipe (AppChat). Omit to drive from ChatSettings. */
  readonly backing?: { readonly handle: string; readonly promptKey: string };
  /** Pin the model id (AppChat). Omit to drive from ChatSettings (the picker). */
  readonly modelId?: string;
  /** Force agent mode on/off. Omit for the interactive toggle (standalone). */
  readonly agentMode?: boolean;
  /** Force a grounding dataset (rare). Omit for the interactive picker. */
  readonly dataset?: string;
  /** Persist threads to the client-local per-endpoint history (standalone only). */
  readonly autosave?: boolean;
  /** App-fixed context bundle handles attached to every turn (AppChat). */
  readonly contextRefs?: readonly string[];
}

export interface ChatController {
  readonly chat: UseChat;
  readonly settings: ChatSettings;
  readonly updateSettings: (next: ChatSettings) => void;
  readonly agentMode: boolean;
  readonly setAgentMode: (v: boolean) => void;
  readonly agentAvailable: boolean;
  readonly agentTurn: boolean;
  readonly dataset: string | undefined;
  readonly setDataset: (v: string | undefined) => void;
  readonly backingHandle: string;
  readonly promptNoModel: boolean;
  readonly attach: ReturnType<typeof useAttachments>;
  readonly contextBundles: ReturnType<typeof useContextBundles>;
  readonly pendingContext: readonly string[];
  readonly toggleContext: (handle: string) => void;
  readonly chatName: string;
  readonly setChatName: (name: string) => void;
  readonly onChatNameInput: (name: string) => void;
  readonly commitName: () => void;
  readonly newChat: () => void;
  readonly loadSaved: (saved: SavedChat) => void;
  readonly exportChat: () => void;
  readonly sendWithAttachments: (text: string) => void;
}

export function useChatController(config: ChatControllerConfig = {}): ChatController {
  const { endpoint } = useConnection();
  const [settings, setSettings] = useState<ChatSettings>(() => loadChatSettings());
  const [interactiveAgentMode, setInteractiveAgentMode] = useState(false);
  const [interactiveDataset, setInteractiveDataset] = useState<string | undefined>(undefined);

  const recipes = useRecipes();
  const available = recipes.data ?? [];
  const agentAvailable = available.includes(REACT_RECIPE_HANDLE);
  const models = useModels();
  const { defaultModelId } = useDefaultModel();
  // POC-5c: when the user has not EXPLICITLY picked a model (config or settings), fall
  // back to the client-local default — but only if it is actually served here, else
  // let the gateway choose (GR15: never send a stale model enum). An explicit pick
  // always wins; the default just fills the gap for new chats.
  const explicitModelId = config.modelId ?? settings.modelId;
  const defaultIsServed =
    defaultModelId !== undefined &&
    (models.models?.some((mm) => mm.modelId === defaultModelId) ?? false);
  const effectiveModelId = explicitModelId ?? (defaultIsServed ? defaultModelId : undefined);
  const chosenModel =
    models.models?.find((mm) => mm.modelId === effectiveModelId) ?? models.models?.[0];

  // A pinned backing (AppChat) wins; else reconcile the persisted handle.
  const backing =
    config.backing ?? resolveChatBacking(settings, available, chosenModel?.chatHandle);

  const promptNoModel = shouldPromptNoModel({
    modelCount: models.models?.length,
    loading: models.loading,
    unsupported: models.unsupported,
    backingHandle: backing.handle,
  });

  // Agent mode + dataset are forced by config or driven interactively.
  const agentMode = config.agentMode ?? interactiveAgentMode;
  const setAgentMode = (v: boolean) => setInteractiveAgentMode(v);
  const dataset = config.dataset ?? interactiveDataset;
  const setDataset = (v: string | undefined) => setInteractiveDataset(v);

  const agentTurn = agentMode && agentAvailable;
  const modelId = effectiveModelId;

  const chat = useChat({
    handle: backing.handle,
    promptKey: backing.promptKey,
    modelId,
    agentMode: agentTurn,
    // RC4b: pass the dataset in BOTH modes — a plain turn grounds via chat-rag, an agent
    // turn routes to react-rag (the model searches it with the `retrieve` tool). The
    // recipe form-gate honest-degrades when react-rag is not provisioned.
    dataset,
    contextRefs: config.contextRefs,
  });

  const attach = useAttachments();
  const contextBundles = useContextBundles();
  const [pendingContext, setPendingContext] = useState<readonly string[]>([]);
  function toggleContext(handle: string): void {
    setPendingContext((prev) =>
      prev.includes(handle) ? prev.filter((h) => h !== handle) : [...prev, handle],
    );
  }

  const autosave = config.autosave ?? true;
  const chatIdRef = useRef<string>(crypto.randomUUID());
  const [chatName, setChatNameState] = useState<string>(() => defaultChatName());
  const chatNameRef = useRef(chatName);
  chatNameRef.current = chatName;
  const userRenamedRef = useRef(false);

  useEffect(() => {
    if (autosave) {
      saveChat(endpoint, chatIdRef.current, chat.thread.messages, chatNameRef.current);
    }
  }, [endpoint, chat.thread, autosave]);

  function updateSettings(next: ChatSettings): void {
    setSettings(next);
    saveChatSettings(next);
  }

  function setChatName(name: string): void {
    setChatNameState(name);
  }

  /** A name EDIT from the input — flips the user-renamed flag (no auto-name). */
  function onChatNameInput(name: string): void {
    userRenamedRef.current = true;
    setChatNameState(name);
  }

  function newChat(): void {
    chatIdRef.current = crypto.randomUUID();
    setChatNameState(defaultChatName());
    userRenamedRef.current = false;
    chat.reset();
  }

  function loadSaved(saved: SavedChat): void {
    chatIdRef.current = saved.id;
    setChatNameState(saved.name ?? saved.title);
    userRenamedRef.current = true;
    chat.loadThread(saved.messages as readonly ChatMessage[]);
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

  function commitName(): void {
    if (autosave && chat.thread.messages.length > 0) {
      renameChat(endpoint, chatIdRef.current, chatName);
    }
  }

  function sendWithAttachments(text: string): void {
    if (!userRenamedRef.current && chat.thread.messages.length === 0) {
      const auto = autoNameFrom([{ id: "seed", role: "user", text, status: "done" }]);
      if (auto) {
        setChatNameState(auto);
        chatNameRef.current = auto;
      }
    }
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

  return {
    chat,
    settings,
    updateSettings,
    agentMode,
    setAgentMode,
    agentAvailable,
    agentTurn,
    dataset,
    setDataset,
    backingHandle: backing.handle,
    promptNoModel,
    attach,
    contextBundles,
    pendingContext,
    toggleContext,
    chatName,
    setChatName,
    onChatNameInput,
    commitName,
    newChat,
    loadSaved,
    exportChat,
    sendWithAttachments,
  };
}
