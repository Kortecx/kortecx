/**
 * Start a run by invoking a published recipe handle with JSON args. Returns the
 * run's hex instance id + terminal (sink) Mote id (no wait) so the caller can route
 * to the live run-detail view, watch it execute, and stop polling authoritatively
 * when the terminal Mote commits. The built-in `kx/recipes/echo` recipe makes this
 * work against a plain `kx serve` with no extra wiring.
 */

import type { Run } from "@kortecx/sdk/web";
import { useMutation } from "@tanstack/react-query";
import { useConnection } from "./connection-context";

export interface InvokeVars {
  handle: string;
  args: Record<string, unknown>;
}

/** The server-derived handles for a started run (all hex; the client never derives an id). */
export interface StartedRun {
  instanceId: string;
  terminalMoteId: string;
  /** The resolved recipe identity — the PR-2.1 fingerprint→handle naming join. */
  recipeFingerprint: string;
}

export function useInvoke() {
  const { client } = useConnection();
  return useMutation<StartedRun, unknown, InvokeVars>({
    mutationFn: async ({ handle, args }) => {
      if (!client) {
        throw new Error("not connected");
      }
      const run = (await client.invoke(handle, args)) as Run;
      return {
        instanceId: run.instanceId,
        terminalMoteId: run.terminalMoteId,
        recipeFingerprint: run.recipeFingerprint,
      };
    },
  });
}
