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

import { type RecipeForm, chainFrom, task } from "@kortecx/sdk/web";
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
import { useSubmitAgentTurn } from "./use-submit-agent-turn";
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

/** AGENTIC-VISION: the image-grounded ReAct loop — agent mode + an attached image route
 *  here so the served VLM reasons over the image on EVERY turn. Provisioned only on a
 *  vision-capable serve with the bundled tool; absent ⇒ agent mode honest-degrades to the
 *  text-only react loop (the image stays display-only). Shares the `kx/recipes/react`
 *  prefix, so it settles as a react CHAIN. */
export const REACT_VISION_RECIPE_HANDLE = "kx/recipes/react-vision";

/** RC4b AGENTIC RAG: the dataset-grounded ReAct loop — agent mode + a selected dataset
 *  route here so the model AUTONOMOUSLY searches the corpus with the read-only `retrieve`
 *  tool (hybrid), reads passages, and can re-query. Provisioned only on an `hnsw` serve;
 *  absent ⇒ agent mode honest-degrades to the plain react loop (the dataset is dropped).
 *  Shares the `kx/recipes/react` prefix, so it settles as a react CHAIN. */
export const REACT_RAG_RECIPE_HANDLE = "kx/recipes/react-rag";

/** RC4b VISION-RAG: image + a selected dataset route here — the served VLM answers about
 *  the image WHILE grounded on the dataset's retrieved text (one generation). Provisioned
 *  only on a vision + `hnsw` serve; absent ⇒ honest-degrade to plain vision (image only). */
export const VISION_RAG_RECIPE_HANDLE = "kx/recipes/vision-rag";

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
  /** POC-1 CHAT-RAG / RC4b: ground each turn over this dataset. In a PLAIN turn the
   *  server embeds → top-k → folds the exact refs (chat-rag). In AGENT mode it routes to
   *  react-rag — the model autonomously searches the corpus with the `retrieve` tool.
   *  `undefined` ⇒ a plain chat. The server/recipe honestly degrades when the dataset is
   *  missing/empty (grounding is never faked). */
  readonly dataset?: string;
  /** POC-5d (AppChat): App-fixed context refs (bundle handles) attached to EVERY
   *  turn, in ADDITION to any per-message context. Optional + additive — existing
   *  callers (the standalone chat) pass nothing and are byte-identical. */
  readonly contextRefs?: readonly string[];
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
    tools?: readonly string[],
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

/** The plan for one turn: either a recipe Invoke (plain / vision / dataset / agent)
 *  or — when tools are attached — a single-MODEL-step agentic workflow submit. */
type TurnPlan =
  | { readonly route: "invoke"; readonly handle: string; readonly args: Record<string, unknown> }
  | {
      readonly route: "agentTools";
      readonly prompt: string;
      readonly toolContract: Record<string, string>;
      readonly maxTurns: number;
      readonly maxToolCalls: number;
    };

/** The bounded-loop budget a tool-attached chat turn requests (mirrors the react
 *  recipe's anchored defaults; the coordinator clamps out-of-range values). */
const TOOL_TURN_MAX_TURNS = 8;
const TOOL_TURN_MAX_TOOL_CALLS = 6;

/**
 * Split the picked `${name}@${version}` tool keys into the `{ name: version }`
 * contract a MODEL step carries (the LAST `@` separates the version; a bare name
 * defaults to `"1"`, the `@tool` grammar default). Pure — unit-testable.
 */
export function toolContractFrom(tools: readonly string[]): Record<string, string> {
  const contract: Record<string, string> = {};
  for (const key of tools) {
    const at = key.lastIndexOf("@");
    if (at <= 0) {
      contract[key] = "1";
    } else {
      // An empty version (a trailing `@`) falls back to the `@tool` grammar default.
      contract[key.slice(0, at)] = key.slice(at + 1) || "1";
    }
  }
  return contract;
}

/**
 * Lower a tool-attached turn to a single-MODEL-step workflow request carrying the
 * tool contract + the bounded-loop budget (as canonical `params`), threaded with
 * the request-level context union. Built with `task` / `chainFrom` (the eager chain
 * surface) — NOT `flow()` (a lazy split chunk that would bloat the eager bundle).
 * Pure — unit-testable.
 */
export function buildAgentTurnRequest(
  prompt: string,
  toolContract: Record<string, string>,
  maxTurns: number,
  maxToolCalls: number,
  modelId: string | undefined,
  context: readonly string[],
) {
  let chain = chainFrom(
    task.model(modelId ?? "", prompt, {}, { tools: toolContract, maxTurns, maxToolCalls }),
  );
  if (context.length > 0) {
    chain = chain.context(...context);
  }
  return chain.build();
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

/**
 * AGENTIC-VISION: build the form-gated arg set for an image-grounded AGENT turn — the
 * react args (`instruction` + budget caps) PLUS the `image_ref` slot — or `null` when the
 * react-vision form lacks either slot (then agent mode honest-degrades to text-only
 * react, the image stays display-only). Pure over the fetched form — unit-testable.
 */
export function planReactVisionArgs(
  form: Pick<RecipeForm, "fields">,
  text: string,
  imageRef: string,
): Record<string, unknown> | null {
  const args = planReactArgs(form, text);
  if (args === null || !form.fields.find((f) => f.name === "image_ref")) {
    return null;
  }
  args.image_ref = imageRef;
  return args;
}

/**
 * RC4b: build the arg set for an agentic-RAG turn — the react args (`instruction` + budget
 * caps) PLUS a `dataset` selector the server strips + folds into the instruction (the model
 * searches it with the `retrieve` tool). `null` when the react-rag form lacks `instruction`
 * (then agent mode honest-degrades to the plain react loop). Pure — unit-testable.
 */
export function planReactRagArgs(
  form: Pick<RecipeForm, "fields">,
  text: string,
  dataset: string,
): Record<string, unknown> | null {
  const args = planReactArgs(form, text);
  if (args === null) {
    return null;
  }
  args.dataset = dataset;
  return args;
}

/**
 * RC4b: build the arg set for a vision-RAG turn — the vision args (`prompt`/`image_ref`/
 * `model`) PLUS a `dataset` + `k` the server strips + folds into the retrieved-text context.
 * `null` when the vision-rag form lacks `image_ref` (then honest-degrade to plain vision).
 */
export function planVisionRagArgs(
  form: Pick<RecipeForm, "fields">,
  text: string,
  imageRef: string,
  modelId: string | undefined,
  dataset: string,
  k: number,
): Record<string, unknown> | null {
  const args = planVisionArgs(form, text, imageRef, modelId);
  if (args === null) {
    return null;
  }
  args.dataset = dataset;
  args.k = k;
  return args;
}

export function useChat({
  handle,
  promptKey,
  modelId,
  agentMode,
  dataset,
  contextRefs,
}: UseChatOptions): UseChat {
  const { client } = useConnection();
  const invoke = useInvoke();
  const submitAgent = useSubmitAgentTurn();
  const [thread, dispatch] = useReducer(chatReducer, EMPTY_THREAD);
  const [active, setActive] = useState<ActiveTurn | null>(null);
  const [degraded, setDegraded] = useState<UiError | null>(null);
  // The assistant id whose result we've already finalized (no double-fetch).
  const finalizedRef = useRef<string | null>(null);
  // The vision form, fetched once per session (`null` = probed and absent).
  const visionFormRef = useRef<RecipeForm | null | undefined>(undefined);
  // The react form likewise (agent mode probes the declared slots).
  const reactFormRef = useRef<RecipeForm | null | undefined>(undefined);
  // AGENTIC-VISION: the react-vision form (agent mode + an image probes it; `null` = the
  // serve has no vision model ⇒ agent mode runs text-only, the image stays display-only).
  const reactVisionFormRef = useRef<RecipeForm | null | undefined>(undefined);
  // RC4b: the react-rag + vision-rag forms (agent/image + a dataset probe them; `null` =
  // the serve has no hnsw datasets ⇒ honest-degrade to plain react / plain vision).
  const reactRagFormRef = useRef<RecipeForm | null | undefined>(undefined);
  const visionRagFormRef = useRef<RecipeForm | null | undefined>(undefined);

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

  /** The react-vision recipe's form, probed once (absent ⇒ agent mode runs text-only). */
  const reactVisionForm = useCallback(async (): Promise<RecipeForm | null> => {
    if (reactVisionFormRef.current !== undefined) {
      return reactVisionFormRef.current;
    }
    try {
      reactVisionFormRef.current = client
        ? await client.getRecipeForm(REACT_VISION_RECIPE_HANDLE)
        : null;
    } catch {
      reactVisionFormRef.current = null; // no vision model — image stays display-only
    }
    return reactVisionFormRef.current;
  }, [client]);

  /** RC4b: the react-rag recipe's form, probed once (absent ⇒ agent mode drops the dataset
   *  and runs the plain react loop). */
  const reactRagForm = useCallback(async (): Promise<RecipeForm | null> => {
    if (reactRagFormRef.current !== undefined) {
      return reactRagFormRef.current;
    }
    try {
      reactRagFormRef.current = client ? await client.getRecipeForm(REACT_RAG_RECIPE_HANDLE) : null;
    } catch {
      reactRagFormRef.current = null; // no hnsw datasets — plain react handles the turn
    }
    return reactRagFormRef.current;
  }, [client]);

  /** RC4b: the vision-rag recipe's form, probed once (absent ⇒ image + dataset degrades to
   *  plain vision, the image answer is ungrounded). */
  const visionRagForm = useCallback(async (): Promise<RecipeForm | null> => {
    if (visionRagFormRef.current !== undefined) {
      return visionRagFormRef.current;
    }
    try {
      visionRagFormRef.current = client
        ? await client.getRecipeForm(VISION_RAG_RECIPE_HANDLE)
        : null;
    } catch {
      visionRagFormRef.current = null; // no vision+hnsw — plain vision handles the turn
    }
    return visionRagFormRef.current;
  }, [client]);

  /** Plan the turn: agent task (+ image → vision agent) → react loop; image → vision; else chat. */
  const planTurn = useCallback(
    async (
      text: string,
      attachments: readonly MessageAttachment[],
      tools: readonly string[],
    ): Promise<TurnPlan> => {
      // A picked tool set routes the turn to a bounded agentic tool loop (a single
      // MODEL step carrying the tool contract) — highest precedence, overriding the
      // vision / dataset / agent routes for THIS turn (a tool turn does not compose).
      if (tools.length > 0) {
        return {
          route: "agentTools",
          prompt: text,
          toolContract: toolContractFrom(tools),
          maxTurns: TOOL_TURN_MAX_TURNS,
          maxToolCalls: TOOL_TURN_MAX_TOOL_CALLS,
        };
      }
      if (agentMode) {
        const image = attachments.find((a) => a.mediaType.startsWith("image/"));
        // AGENTIC-VISION: agent mode + an attached image → the image-grounded react loop,
        // so the served VLM reasons over the image on EVERY turn. Form-gated; absent ⇒
        // fall through to the text-only react loop (the image stays display-only).
        if (image) {
          const vform = await reactVisionForm();
          if (vform) {
            const args = planReactVisionArgs(vform, text, image.ref);
            if (args !== null) {
              return { route: "invoke", handle: REACT_VISION_RECIPE_HANDLE, args };
            }
          }
        }
        // RC4b: agent mode + a selected dataset (no image) → the agentic-RAG loop, so the
        // model searches the corpus with the `retrieve` tool. Form-gated; absent ⇒ fall
        // through to the plain react loop (the dataset is dropped, honestly).
        if (dataset && !image) {
          const rform = await reactRagForm();
          if (rform) {
            const args = planReactRagArgs(rform, text, dataset);
            if (args !== null) {
              return { route: "invoke", handle: REACT_RAG_RECIPE_HANDLE, args };
            }
          }
        }
        const form = await reactForm();
        if (form) {
          const args = planReactArgs(form, text);
          if (args !== null) {
            return { route: "invoke", handle: REACT_RECIPE_HANDLE, args };
          }
        }
      }
      const image = attachments.find((a) => a.mediaType.startsWith("image/"));
      if (image) {
        // RC4b: image + a selected dataset → vision-RAG (the VLM answers about the image
        // grounded on the dataset's retrieved text). Form-gated; absent ⇒ plain vision.
        if (dataset) {
          const vrform = await visionRagForm();
          if (vrform) {
            const args = planVisionRagArgs(
              vrform,
              text,
              image.ref,
              modelId,
              dataset,
              CHAT_RAG_DEFAULT_K,
            );
            if (args !== null) {
              return { route: "invoke", handle: VISION_RAG_RECIPE_HANDLE, args };
            }
          }
        }
        const form = await visionForm();
        if (form) {
          const args = planVisionArgs(form, text, image.ref, modelId);
          if (args !== null) {
            return { route: "invoke", handle: VISION_RECIPE_HANDLE, args };
          }
        }
      }
      // POC-1 CHAT-RAG: a selected dataset grounds the turn — route to the chat-rag
      // recipe carrying the dataset selector (the server strips it + folds the
      // retrieved refs; a missing/empty dataset degrades to a plain answer).
      if (dataset) {
        return {
          route: "invoke",
          handle: CHAT_RAG_RECIPE_HANDLE,
          args: { [promptKey]: text, dataset, k: CHAT_RAG_DEFAULT_K },
        };
      }
      return { route: "invoke", handle, args: { [promptKey]: text } };
    },
    [
      handle,
      promptKey,
      modelId,
      agentMode,
      dataset,
      reactForm,
      reactRagForm,
      reactVisionForm,
      visionForm,
      visionRagForm,
    ],
  );

  const startTurn = useCallback(
    async (
      assistantId: string,
      text: string,
      attachments: readonly MessageAttachment[],
      context: readonly string[],
      tools: readonly string[],
    ): Promise<void> => {
      try {
        const plan = await planTurn(text, attachments, tools);
        // PR-7b: context is request-level — it attaches to the run regardless of
        // which route (chat / vision / react / tool loop) the plan picked. POC-5d:
        // App-fixed `contextRefs` ride EVERY turn, unioned with per-message
        // context (dedup-stable; existing callers pass none ⇒ unchanged).
        const merged =
          contextRefs && contextRefs.length > 0
            ? [...new Set([...contextRefs, ...context])]
            : context;
        // A retried assistant id must be re-finalizable.
        if (finalizedRef.current === assistantId) {
          finalizedRef.current = null;
        }
        if (plan.route === "agentTools") {
          // A tool-attached turn submits a single-MODEL-step workflow and waits by
          // the run's `react_chain_salt` (exactly-once) — the SAME salt-scoped
          // ListReactTurns poll an agent turn uses; there is no static terminal Mote.
          const request = buildAgentTurnRequest(
            plan.prompt,
            plan.toolContract,
            plan.maxTurns,
            plan.maxToolCalls,
            modelId,
            merged,
          );
          const { instanceId, reactChainSalt } = await submitAgent.mutateAsync(request);
          if (reactChainSalt === "") {
            // The gateway accepted the workflow but did not scope this tool-carrying
            // MODEL step as an agentic chain, so its answer can't be located exactly-once.
            // Fail honestly rather than hang waiting for react rounds that never arrive.
            dispatch({
              type: "turn_failed",
              assistantId,
              error: {
                code: "tool_turn_unscoped",
                kind: "generic",
                title: "Tool turn not supported here",
                message:
                  "This gateway ran the turn but did not scope it for tool use. Send again without tools attached.",
                retryable: false,
              },
            });
            return;
          }
          dispatch({ type: "turn_started", assistantId, instanceId, terminalMoteId: "" });
          setActive({ assistantId, instanceId, terminalMoteId: "", react: true, reactChainSalt });
          return;
        }
        const { instanceId, terminalMoteId, reactChainSalt } = await invoke.mutateAsync({
          handle: plan.handle,
          args: plan.args,
          context: merged.length > 0 ? merged : undefined,
        });
        dispatch({ type: "turn_started", assistantId, instanceId, terminalMoteId });
        setActive({
          assistantId,
          instanceId,
          terminalMoteId,
          // AGENTIC-VISION: react + react-vision both settle as react CHAINS (prefix).
          react: plan.handle.startsWith(REACT_RECIPE_HANDLE),
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
    [invoke, submitAgent, planTurn, contextRefs, modelId],
  );

  const send = useCallback(
    async (
      text: string,
      attachments: readonly MessageAttachment[] = [],
      context: readonly string[] = [],
      tools: readonly string[] = [],
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
        tools: tools.length > 0 ? tools : undefined,
      });
      await startTurn(assistantId, trimmed, attachments, context, tools);
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
      await startTurn(assistantId, source.text, source.attachments, source.context, source.tools);
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
