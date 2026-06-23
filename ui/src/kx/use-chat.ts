/**
 * The agentic-chat orchestrator. One user message → one runtime run:
 *   Invoke(handle, { [promptKey]: text }) → poll the projection (existing poll-stop
 *   on the terminal Mote) → on terminal COMMIT fetch + decode the result → render.
 *
 * Batch A adds attachments + the vision route, FORM-GATED: an image rides as a
 * `kx/recipes/vision` invoke ONLY when that recipe's published form declares the
 * `image_ref` slot (we NEVER send an undeclared arg — Invoke binding is
 * fail-closed). Without the vision recipe the attachment stays display-only on
 * the user bubble. The picked model likewise only rides when the form declares a
 * `model` ENUM (the server validates the value — SN-8). A FAILED turn retries
 * with its IDENTICAL args: refs are content-addressed and the runtime dedups,
 * so the retry either re-runs or joins the existing run.
 *
 * This hook owns the I/O only; the thread state lives in the pure `chatReducer`.
 */

import type { RecipeForm } from "@kortecx/sdk/web";
import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import {
  type ChatMessage,
  type ChatThread,
  EMPTY_THREAD,
  type MessageAttachment,
  chatReducer,
  isTurnInFlight,
  retrySource,
} from "../lib/chat-thread";
import { decodeContent } from "../lib/content-decode";
import { useConnection } from "./connection-context";
import { type UiError, toUiError } from "./errors";
import { useInvoke } from "./use-invoke";
import { type ProjectionVM, runSettled, useProjection } from "./use-projection";
import { type ReactTurnVM, useReactProgress } from "./use-react-progress";
import { useTokenStream } from "./use-token-stream";

const COMMITTED = 3;

/** The Batch A vision recipe (provisioned only on an image-capable serve). */
export const VISION_RECIPE_HANDLE = "kx/recipes/vision";

/** POC-1 CHAT-RAG: the AUTO-RAG chat recipe (provisioned on any inference serve).
 *  A turn routed here carries a `dataset` arg; the server embeds the prompt,
 *  retrieves the dataset's top-k docs, and folds the exact refs into the prompt
 *  (honest plain chat when the dataset is missing/empty — grounding is never faked). */
export const CHAT_RAG_RECIPE_HANDLE = "kx/recipes/chat-rag";

/** POC-1: the default number of grounding documents folded into a chat-rag turn
 *  (mirrors the server-side default; the server clamps an out-of-range value). */
const CHAT_RAG_DEFAULT_K = 4;

/** The live ReAct task-loop recipe (provisioned only on an inference serve
 *  with the bundled tool — the PR-2.1 agent mode's backend). */
export const REACT_RECIPE_HANDLE = "kx/recipes/react";

/** The PR-6b-4 auto-grant ReAct loop — provisioned only when the operator opts
 *  in via `KX_SERVE_AUTOGRANT`. Its presence in `ListRecipes` is the honest
 *  ON/OFF signal the Tools section reflects (the runtime is the source of truth;
 *  OSS exposes no toggle — enabling it is an operator/Cloud concern). */
export const REACT_AUTO_RECIPE_HANDLE = "kx/recipes/react-auto";

interface ActiveTurn {
  readonly assistantId: string;
  readonly instanceId: string;
  readonly terminalMoteId: string;
  /** An agent (react) turn — resolved via ListReactTurns, never the terminal
   *  projection settle (the seed Mote is SWAPPED; the chain extends). */
  readonly react: boolean;
  /** PR-R1: the per-invocation react chain key — scopes this turn's progress to
   *  ITS chain on serve's shared journal (every chat turn shares one instanceId). */
  readonly reactChainSalt: string;
}

export interface UseChatOptions {
  /** Recipe handle that backs chat (from chat settings). */
  readonly handle: string;
  /** Free-param key the message binds to (from chat settings). */
  readonly promptKey: string;
  /** The picked model id (rides only when a form declares a `model` ENUM). */
  readonly modelId?: string;
  /** Agent mode (PR-2.1): route the message as a TASK to the react loop —
   *  the model reasons + fires tools until it answers. Only honored when the
   *  react recipe is provisioned (the caller gates the toggle). */
  readonly agentMode?: boolean;
  /** POC-1 CHAT-RAG: ground each turn over this dataset (embed → top-k → fold the
   *  exact refs). `undefined` ⇒ a plain chat. Ignored in agent mode (the loop has
   *  its own context carry). The server honestly degrades to a plain answer when
   *  the dataset is missing/empty. */
  readonly dataset?: string;
}

export interface UseChat {
  readonly thread: ChatThread;
  /** True while a turn is in flight (disable the composer). */
  readonly busy: boolean;
  /** Set when the gateway has no usable chat recipe/model (show guidance). */
  readonly degraded: UiError | null;
  /** The in-flight run's projection (for the DAG-of-thought), if any. */
  readonly activeProjection: ProjectionVM | undefined;
  /** The assistant message id the active projection belongs to. */
  readonly activeAssistantId: string | undefined;
  /** The in-flight AGENT turn's loop progress (react turns), if any. */
  readonly reactTurns: readonly ReactTurnVM[] | undefined;
  send(
    text: string,
    attachments?: readonly MessageAttachment[],
    context?: readonly string[],
  ): Promise<void>;
  /** Re-dispatch a FAILED turn with its identical args. */
  retry(assistantId: string): Promise<void>;
  /** Restore a saved thread (chat history) — replaces the live one. */
  loadThread(messages: readonly ChatMessage[]): void;
  reset(): void;
}

/** Recipe-absence signals: the gateway is up but cannot run this chat handle. */
function isDegradeError(ui: UiError): boolean {
  return ui.kind === "not-wired" || ui.kind === "forbidden" || ui.kind === "not-found";
}

const TERMINAL_FAILED: UiError = {
  code: "run_failed",
  kind: "generic",
  title: "Run failed",
  message: "The chat run's terminal step did not commit.",
  retryable: false,
};

/** The invoke plan for one turn: which recipe, with which (form-gated) args. */
interface TurnPlan {
  readonly handle: string;
  readonly args: Record<string, unknown>;
}

/**
 * Build the form-gated arg set for a vision turn, or `null` when the vision
 * form doesn't declare the `image_ref` slot (then the attachment is
 * display-only). Pure over the fetched form — unit-testable.
 */
/**
 * Build the form-gated arg set for an AGENT (react) turn, or `null` when the
 * react form doesn't declare the `instruction` slot. Budget caps ride only
 * when declared (8 turns / 6 tool calls — the recipe's anchored defaults).
 * Pure over the fetched form — unit-testable.
 */
export function planReactArgs(
  form: Pick<RecipeForm, "fields">,
  text: string,
): Record<string, unknown> | null {
  const byName = (n: string) => form.fields.find((f) => f.name === n);
  if (!byName("instruction")) {
    return null;
  }
  const args: Record<string, unknown> = { instruction: text };
  if (byName("max_turns")) {
    args.max_turns = 8;
  }
  if (byName("max_tool_calls")) {
    args.max_tool_calls = 6;
  }
  return args;
}

export function planVisionArgs(
  form: Pick<RecipeForm, "fields">,
  text: string,
  imageRef: string,
  modelId: string | undefined,
): Record<string, unknown> | null {
  const byName = (n: string) => form.fields.find((f) => f.name === n);
  if (!byName("image_ref")) {
    return null;
  }
  const args: Record<string, unknown> = { image_ref: imageRef };
  if (byName("prompt")) {
    args.prompt = text;
  }
  const model = byName("model");
  if (model) {
    // The server validates ENUM membership; we pre-pick a legal value so the
    // happy path never round-trips a refusal.
    args.model =
      modelId !== undefined && model.allowed.includes(modelId) ? modelId : model.allowed[0];
  }
  return args;
}

export function useChat({
  handle,
  promptKey,
  modelId,
  agentMode,
  dataset,
}: UseChatOptions): UseChat {
  const { client } = useConnection();
  const invoke = useInvoke();
  const [thread, dispatch] = useReducer(chatReducer, EMPTY_THREAD);
  const [active, setActive] = useState<ActiveTurn | null>(null);
  const [degraded, setDegraded] = useState<UiError | null>(null);
  // The assistant id whose result we've already finalized (no double-fetch).
  const finalizedRef = useRef<string | null>(null);
  // The vision form, fetched once per session (`null` = probed and absent).
  const visionFormRef = useRef<RecipeForm | null | undefined>(undefined);
  // The react form likewise (agent mode probes the declared slots).
  const reactFormRef = useRef<RecipeForm | null | undefined>(undefined);

  const projection = useProjection(
    active?.instanceId,
    active?.terminalMoteId ? { terminalMoteId: active.terminalMoteId } : {},
  );

  // Agent turns: the loop's durable facts narrate progress + completion.
  const reactProgress = useReactProgress(
    active?.react ? active.instanceId : undefined,
    active?.react ? active.reactChainSalt : undefined,
  );

  // PR-4.2 (T-STREAM1): the live token stream for the in-flight mote. For a
  // simple/vision turn it's the terminal mote (streams into the answer bubble);
  // for an agent chain it's the LATEST in-flight turn (streams into the reasoning
  // trace, so a tool turn's raw envelope never poses as the answer). The committed
  // result stays the authority and overwrites the stream on settle.
  const reactTurnList = reactProgress.turns;
  const latestReactMote = reactTurnList[reactTurnList.length - 1]?.turnMoteId;
  const streamMoteId = active
    ? active.react
      ? latestReactMote
      : active.terminalMoteId
    : undefined;
  const tokenStream = useTokenStream(active?.instanceId, streamMoteId, Boolean(active));

  const streamText = tokenStream.text;
  const streamAssistantId = active?.assistantId;
  const streamTarget: "answer" | "reasoning" = active?.react ? "reasoning" : "answer";
  useEffect(() => {
    if (!streamAssistantId || streamText === "") {
      return;
    }
    dispatch({
      type: "token_streamed",
      assistantId: streamAssistantId,
      text: streamText,
      target: streamTarget,
    });
  }, [streamAssistantId, streamText, streamTarget]);

  // When the active AGENT chain settles, resolve the answer turn's committed
  // text (one imperative projection read — the poll hook's at-rest heuristic
  // would stall BETWEEN turns, so the facts drive completion, not the poll).
  useEffect(() => {
    if (!active?.react || !client) {
      return;
    }
    const terminal = reactProgress.terminal;
    if (!terminal || finalizedRef.current === active.assistantId) {
      return;
    }
    finalizedRef.current = active.assistantId;
    void (async () => {
      try {
        if (terminal.branch !== "answer") {
          dispatch({
            type: "turn_failed",
            assistantId: active.assistantId,
            error: {
              code: "run_failed",
              kind: "generic",
              title: "Agent task dead-lettered",
              message: `The loop ended without an answer (turn ${terminal.turn}).`,
              retryable: false,
            },
          });
          return;
        }
        const view = await client.getProjection(active.instanceId);
        const answer = view.motes.find((m) => m.moteId === terminal.turnMoteId);
        if (answer?.resultRef) {
          const bytes = await client.getContent(answer.resultRef, active.instanceId);
          const decoded = decodeContent(bytes);
          dispatch({
            type: "turn_done",
            assistantId: active.assistantId,
            text: decoded.text === "" ? "(empty answer)" : decoded.text,
          });
        } else {
          dispatch({
            type: "turn_done",
            assistantId: active.assistantId,
            text: "(answered with no output)",
          });
        }
      } catch (e) {
        dispatch({ type: "turn_failed", assistantId: active.assistantId, error: toUiError(e) });
      } finally {
        setActive(null);
      }
    })();
  }, [active, reactProgress.terminal, client]);

  // When the active CHAT run settles, fetch + decode the terminal result once.
  useEffect(() => {
    if (!active || active.react || !client) {
      return;
    }
    const data = projection.data;
    if (!data || !runSettled(data, active.terminalMoteId)) {
      return;
    }
    if (finalizedRef.current === active.assistantId) {
      return;
    }
    finalizedRef.current = active.assistantId;
    const term = data.motes.find((m) => m.moteId === active.terminalMoteId);

    void (async () => {
      try {
        if (!term || term.stateCode !== COMMITTED) {
          dispatch({
            type: "turn_failed",
            assistantId: active.assistantId,
            error: TERMINAL_FAILED,
          });
        } else if (term.resultRef) {
          const bytes = await client.getContent(term.resultRef, active.instanceId);
          const decoded = decodeContent(bytes);
          dispatch({
            type: "turn_done",
            assistantId: active.assistantId,
            text: decoded.text === "" ? "(empty result)" : decoded.text,
          });
        } else {
          dispatch({
            type: "turn_done",
            assistantId: active.assistantId,
            text: "(committed with no output)",
          });
        }
      } catch (e) {
        dispatch({ type: "turn_failed", assistantId: active.assistantId, error: toUiError(e) });
      } finally {
        setActive(null);
      }
    })();
  }, [active, projection.data, client]);

  /** The vision recipe's form, probed once (absent/old gateways yield `null`). */
  const visionForm = useCallback(async (): Promise<RecipeForm | null> => {
    if (visionFormRef.current !== undefined) {
      return visionFormRef.current;
    }
    try {
      visionFormRef.current = client ? await client.getRecipeForm(VISION_RECIPE_HANDLE) : null;
    } catch {
      visionFormRef.current = null; // not provisioned / old gateway — display-only attachments
    }
    return visionFormRef.current;
  }, [client]);

  /** The react recipe's form, probed once (absent ⇒ agent mode falls back). */
  const reactForm = useCallback(async (): Promise<RecipeForm | null> => {
    if (reactFormRef.current !== undefined) {
      return reactFormRef.current;
    }
    try {
      reactFormRef.current = client ? await client.getRecipeForm(REACT_RECIPE_HANDLE) : null;
    } catch {
      reactFormRef.current = null; // not provisioned — chat handles the turn
    }
    return reactFormRef.current;
  }, [client]);

  /** Plan the turn: agent task → react loop; image → vision; else plain chat. */
  const planTurn = useCallback(
    async (text: string, attachments: readonly MessageAttachment[]): Promise<TurnPlan> => {
      if (agentMode) {
        const form = await reactForm();
        if (form) {
          const args = planReactArgs(form, text);
          if (args !== null) {
            return { handle: REACT_RECIPE_HANDLE, args };
          }
        }
      }
      const image = attachments.find((a) => a.mediaType.startsWith("image/"));
      if (image) {
        const form = await visionForm();
        if (form) {
          const args = planVisionArgs(form, text, image.ref, modelId);
          if (args !== null) {
            return { handle: VISION_RECIPE_HANDLE, args };
          }
        }
      }
      // POC-1 CHAT-RAG: a selected dataset grounds the turn — route to the chat-rag
      // recipe carrying the dataset selector (the server strips it + folds the
      // retrieved refs; a missing/empty dataset degrades to a plain answer).
      if (dataset) {
        return {
          handle: CHAT_RAG_RECIPE_HANDLE,
          args: { [promptKey]: text, dataset, k: CHAT_RAG_DEFAULT_K },
        };
      }
      return { handle, args: { [promptKey]: text } };
    },
    [handle, promptKey, modelId, agentMode, dataset, reactForm, visionForm],
  );

  const startTurn = useCallback(
    async (
      assistantId: string,
      text: string,
      attachments: readonly MessageAttachment[],
      context: readonly string[],
    ): Promise<void> => {
      try {
        const plan = await planTurn(text, attachments);
        // PR-7b: context is request-level — it attaches to the run regardless of
        // which recipe route (chat / vision / react) the plan picked.
        const { instanceId, terminalMoteId, reactChainSalt } = await invoke.mutateAsync({
          ...plan,
          context: context.length > 0 ? context : undefined,
        });
        dispatch({ type: "turn_started", assistantId, instanceId, terminalMoteId });
        // A retried assistant id must be re-finalizable.
        if (finalizedRef.current === assistantId) {
          finalizedRef.current = null;
        }
        setActive({
          assistantId,
          instanceId,
          terminalMoteId,
          react: plan.handle === REACT_RECIPE_HANDLE,
          reactChainSalt,
        });
      } catch (e) {
        const ui = toUiError(e);
        dispatch({ type: "turn_failed", assistantId, error: ui });
        if (isDegradeError(ui)) {
          setDegraded(ui);
        }
      }
    },
    [invoke, planTurn],
  );

  const send = useCallback(
    async (
      text: string,
      attachments: readonly MessageAttachment[] = [],
      context: readonly string[] = [],
    ): Promise<void> => {
      const trimmed = text.trim();
      if (trimmed === "") {
        return;
      }
      const userId = crypto.randomUUID();
      const assistantId = crypto.randomUUID();
      setDegraded(null);
      dispatch({
        type: "user_send",
        userId,
        assistantId,
        text: trimmed,
        attachments: attachments.length > 0 ? attachments : undefined,
        context: context.length > 0 ? context : undefined,
      });
      await startTurn(assistantId, trimmed, attachments, context);
    },
    [startTurn],
  );

  const retry = useCallback(
    async (assistantId: string): Promise<void> => {
      const source = retrySource(thread, assistantId);
      if (!source) {
        return;
      }
      setDegraded(null);
      dispatch({ type: "turn_retry", assistantId });
      await startTurn(assistantId, source.text, source.attachments, source.context);
    },
    [thread, startTurn],
  );

  const loadThread = useCallback((messages: readonly ChatMessage[]): void => {
    setActive(null);
    setDegraded(null);
    finalizedRef.current = null;
    dispatch({ type: "load_thread", messages });
  }, []);

  const reset = useCallback((): void => {
    setActive(null);
    setDegraded(null);
    finalizedRef.current = null;
    dispatch({ type: "reset" });
  }, []);

  return {
    thread,
    busy: isTurnInFlight(thread),
    degraded,
    activeProjection: active ? projection.data : undefined,
    activeAssistantId: active?.assistantId,
    reactTurns: active?.react ? reactProgress.turns : undefined,
    send,
    retry,
    loadThread,
    reset,
  };
}
