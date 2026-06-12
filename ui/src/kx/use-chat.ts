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

const COMMITTED = 3;

/** The Batch A vision recipe (provisioned only on an image-capable serve). */
export const VISION_RECIPE_HANDLE = "kx/recipes/vision";

interface ActiveTurn {
  readonly assistantId: string;
  readonly instanceId: string;
  readonly terminalMoteId: string;
}

export interface UseChatOptions {
  /** Recipe handle that backs chat (from chat settings). */
  readonly handle: string;
  /** Free-param key the message binds to (from chat settings). */
  readonly promptKey: string;
  /** The picked model id (rides only when a form declares a `model` ENUM). */
  readonly modelId?: string;
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
  send(text: string, attachments?: readonly MessageAttachment[]): Promise<void>;
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

export function useChat({ handle, promptKey, modelId }: UseChatOptions): UseChat {
  const { client } = useConnection();
  const invoke = useInvoke();
  const [thread, dispatch] = useReducer(chatReducer, EMPTY_THREAD);
  const [active, setActive] = useState<ActiveTurn | null>(null);
  const [degraded, setDegraded] = useState<UiError | null>(null);
  // The assistant id whose result we've already finalized (no double-fetch).
  const finalizedRef = useRef<string | null>(null);
  // The vision form, fetched once per session (`null` = probed and absent).
  const visionFormRef = useRef<RecipeForm | null | undefined>(undefined);

  const projection = useProjection(
    active?.instanceId,
    active?.terminalMoteId ? { terminalMoteId: active.terminalMoteId } : {},
  );

  // When the active run settles, fetch + decode the terminal result once.
  useEffect(() => {
    if (!active || !client) {
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

  /** Plan the turn: the vision route when an image ref can bind, else plain chat. */
  const planTurn = useCallback(
    async (text: string, attachments: readonly MessageAttachment[]): Promise<TurnPlan> => {
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
      return { handle, args: { [promptKey]: text } };
    },
    [handle, promptKey, modelId, visionForm],
  );

  const startTurn = useCallback(
    async (
      assistantId: string,
      text: string,
      attachments: readonly MessageAttachment[],
    ): Promise<void> => {
      try {
        const plan = await planTurn(text, attachments);
        const { instanceId, terminalMoteId } = await invoke.mutateAsync(plan);
        dispatch({ type: "turn_started", assistantId, instanceId, terminalMoteId });
        // A retried assistant id must be re-finalizable.
        if (finalizedRef.current === assistantId) {
          finalizedRef.current = null;
        }
        setActive({ assistantId, instanceId, terminalMoteId });
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
    async (text: string, attachments: readonly MessageAttachment[] = []): Promise<void> => {
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
      });
      await startTurn(assistantId, trimmed, attachments);
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
      await startTurn(assistantId, source.text, source.attachments);
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
    send,
    retry,
    loadThread,
    reset,
  };
}
