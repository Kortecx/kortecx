/**
 * The agentic-chat orchestrator. One user message → one runtime run:
 *   Invoke(handle, { [promptKey]: text }) → poll the projection (existing poll-stop
 *   on the terminal Mote) → on terminal COMMIT fetch + decode the result → render.
 *
 * This hook owns the I/O only; the thread state lives in the pure `chatReducer`. The
 * in-flight run's projection is exposed so the panel can render the DAG-of-thought.
 * It degrades gracefully when no chat recipe/model is provisioned: the authoritative
 * signal is the Invoke error code (the signature catalog ≠ the recipe library, so a
 * listing probe is unreliable — we branch on the gRPC error instead).
 */

import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { type ChatThread, EMPTY_THREAD, chatReducer, isTurnInFlight } from "../lib/chat-thread";
import { decodeContent } from "../lib/content-decode";
import { useConnection } from "./connection-context";
import { type UiError, toUiError } from "./errors";
import { useInvoke } from "./use-invoke";
import { type ProjectionVM, runSettled, useProjection } from "./use-projection";

const COMMITTED = 3;

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
  send(text: string): Promise<void>;
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

export function useChat({ handle, promptKey }: UseChatOptions): UseChat {
  const { client } = useConnection();
  const invoke = useInvoke();
  const [thread, dispatch] = useReducer(chatReducer, EMPTY_THREAD);
  const [active, setActive] = useState<ActiveTurn | null>(null);
  const [degraded, setDegraded] = useState<UiError | null>(null);
  // The assistant id whose result we've already finalized (no double-fetch).
  const finalizedRef = useRef<string | null>(null);

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

  const send = useCallback(
    async (text: string): Promise<void> => {
      const trimmed = text.trim();
      if (trimmed === "") {
        return;
      }
      const userId = crypto.randomUUID();
      const assistantId = crypto.randomUUID();
      setDegraded(null);
      dispatch({ type: "user_send", userId, assistantId, text: trimmed });
      try {
        const { instanceId, terminalMoteId } = await invoke.mutateAsync({
          handle,
          args: { [promptKey]: trimmed },
        });
        dispatch({ type: "turn_started", assistantId, instanceId, terminalMoteId });
        setActive({ assistantId, instanceId, terminalMoteId });
      } catch (e) {
        const ui = toUiError(e);
        dispatch({ type: "turn_failed", assistantId, error: ui });
        if (isDegradeError(ui)) {
          setDegraded(ui);
        }
      }
    },
    [invoke, handle, promptKey],
  );

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
    reset,
  };
}
