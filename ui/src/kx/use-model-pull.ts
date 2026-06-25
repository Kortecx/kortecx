/**
 * Model Control v2 — `kx models pull` from the console: download + RUNTIME-register
 * a model (no restart). `start({ ollamaTag })` pulls from the Ollama registry;
 * `start({ url, sha256 })` downloads a `huggingface.co` `/resolve/` GGUF. The hook
 * then POLLS `GetPullStatus` to a terminal state, surfacing live byte progress from
 * REAL server facts (never a fabricated bar). On completion it invalidates the models
 * query so the new model appears in the list immediately. Deny-by-default: a refusal
 * (downloads disabled / host not allowlisted / missing sha256) surfaces as `startError`.
 */

import type { PullStatus } from "@kortecx/sdk/web";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

/** A pull phase is terminal once the server reports done or failed. */
function isTerminalPhase(phase: string): boolean {
  return phase === "done" || phase === "failed";
}

export interface UseModelPull {
  /** Kick off a pull (exactly one of `ollamaTag` or `url`+`sha256`). */
  readonly start: (args: { ollamaTag?: string; url?: string; sha256?: string }) => Promise<void>;
  /** The latest polled status (null until a pull starts + a status arrives). */
  readonly status: PullStatus | null;
  /** A start-time refusal / error (deny-by-default reason), or null. */
  readonly startError: string | null;
  /** True while the PullModel call itself is in flight. */
  readonly starting: boolean;
  /** True while a pull is downloading/registering (not yet terminal). */
  readonly active: boolean;
  /** Clear the tracked pull (dismiss a finished/failed status). */
  readonly reset: () => void;
}

export function useModelPull(): UseModelPull {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  const [modelId, setModelId] = useState<string | null>(null);
  const [startError, setStartError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);

  const statusQuery = useQuery({
    queryKey: ["kx", endpoint, "model-pull", modelId],
    enabled: client !== null && modelId !== null,
    // Poll every 0.8s until the server reports a terminal phase.
    refetchInterval: (query) => {
      const s = query.state.data as PullStatus | undefined;
      return s && isTerminalPhase(s.phase) ? false : 800;
    },
    queryFn: async (): Promise<PullStatus> => {
      if (!client || !modelId) {
        throw new Error("no pull in flight");
      }
      const s = await client.getPullStatus(modelId);
      if (isTerminalPhase(s.phase)) {
        // The model appears in ListModels the instant it registers.
        void qc.invalidateQueries({ queryKey: queryKeys.models(endpoint) });
      }
      return s;
    },
  });

  const start = async (args: { ollamaTag?: string; url?: string; sha256?: string }) => {
    if (!client) {
      return;
    }
    setStartError(null);
    setStarting(true);
    setModelId(null);
    try {
      const id = await client.pullModel(args);
      setModelId(id);
    } catch (e) {
      setStartError(toUiError(e).message);
    } finally {
      setStarting(false);
    }
  };

  const reset = () => {
    setModelId(null);
    setStartError(null);
  };

  const status = (statusQuery.data as PullStatus | undefined) ?? null;
  const active = modelId !== null && status !== null && !isTerminalPhase(status.phase);
  return { start, status, startError, starting, active, reset };
}
