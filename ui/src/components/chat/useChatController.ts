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
 *
 * PR-A: the standalone New Chat is now READ-ONLY, RAG-grounded. The
 * interactive Agent-task toggle + the per-turn tool picker + the MCP chips are gone
 * from the standalone surface (mutate-capable agentic chat lives in App chat); this
 * hook keeps only the read-only grounding levers (dataset + context bundles). The
 * agentic path stays for AppChat via `config.agentMode` (an agentic App's react
 * loop) — the capability is relocated, never crippled (Principle 3).
 */

import type { ModelSummary } from "@kortecx/sdk/web";
import { useEffect, useRef, useState } from "react";
import { useConnection } from "../../kx/connection-context";
import { useAttachments } from "../../kx/use-attachments";
import { REACT_RECIPE_HANDLE, type UseChat, useChat } from "../../kx/use-chat";
import { useContextBundles } from "../../kx/use-context-bundles";
import { useDefaultModel } from "../../kx/use-default-model";
import { useModels } from "../../kx/use-models";
import { useRecipes } from "../../kx/use-recipes";
import { resolveBoundModel } from "../../lib/auto-model";
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
  /** Force agent mode on (an agentic App's react loop). Omit ⇒ read-only chat
   *  (the standalone New Chat never enables the mutate-capable agent path). */
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
  /** True when THIS turn runs the agent loop (AppChat's agentic apps). The
   *  standalone read-only chat never enables it. */
  readonly agentTurn: boolean;
  readonly dataset: string | undefined;
  readonly setDataset: (v: string | undefined) => void;
  readonly backingHandle: string;
  /** The model this turn actually BINDS (undefined ⇒ nothing served / still loading).
   *  Surfaces read it instead of re-deriving from `settings.modelId` — re-deriving is
   *  what let the label and the bound model drift apart. */
  readonly boundModel: ModelSummary | undefined;
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
  const [interactiveDataset, setInteractiveDataset] = useState<string | undefined>(undefined);

  const recipes = useRecipes();
  const available = recipes.data ?? [];
  const agentAvailable = available.includes(REACT_RECIPE_HANDLE);
  const models = useModels();
  const { defaultModelId } = useDefaultModel();
  // The ONE resolution both this hook and the ModelPicker derive from, so the id a
  // turn sends and the "Auto · X" the picker labels cannot diverge. An explicit pick
  // (config = the AppChat pin, else the picker's persisted choice) is honored only
  // when this serve actually serves it; otherwise Auto binds — server-active first
  // (Model Control v2), then a served client-local default.
  //
  // The unserved-pick case is why `chosenModel` no longer falls back to `models[0]`:
  // plain chat routes by `chatHandle` ALONE, so that fallback silently ran the turn
  // on models[0] while the picker promised the resolved model. Taking both the id and
  // the handle off one `ModelSummary` makes the two disagree-proof.
  const bound = resolveBoundModel(
    models.models,
    config.modelId ?? settings.modelId,
    defaultModelId,
  );
  const chosenModel = bound.model;

  // A pinned backing (AppChat) wins; else reconcile the persisted handle.
  const backing =
    config.backing ?? resolveChatBacking(settings, available, chosenModel?.chatHandle);

  const promptNoModel = shouldPromptNoModel({
    modelCount: models.models?.length,
    loading: models.loading,
    unsupported: models.unsupported,
    backingHandle: backing.handle,
  });

  // Agent mode is forced by config only (AppChat's agentic apps). The read-only
  // standalone chat never enables it — there is no interactive toggle. The dataset
  // is forced by config or driven interactively (the grounding bar).
  const agentMode = config.agentMode ?? false;
  const dataset = config.dataset ?? interactiveDataset;
  const setDataset = (v: string | undefined) => setInteractiveDataset(v);

  const agentTurn = agentMode && agentAvailable;
  // Always a SERVED id or undefined — never a stale enum (GR15). Undefined lets the
  // gateway resolve the default itself (SN-8), which is the honest answer while the
  // model list is still loading.
  const modelId = chosenModel?.modelId;

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
    // Read-only turn: no per-message tools are attached (the mutate-capable tool
    // picker is gone from the standalone chat — grounding is dataset + context).
    void chat.send(text, ready, pendingContext, []);
    attach.clear();
    setPendingContext([]);
  }

  return {
    chat,
    settings,
    updateSettings,
    agentTurn,
    dataset,
    setDataset,
    backingHandle: backing.handle,
    boundModel: chosenModel,
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
