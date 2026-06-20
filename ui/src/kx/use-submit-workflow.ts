/**
 * Author + run a Tier-1 DAG via `SubmitWorkflow` (the visual builder's submit
 * path). The client sends ONLY topology + params (the SDK `BlueprintBuilder`
 * shape) — the SERVER compiles the DAG, derives every identity, and builds every
 * per-step warrant from the party's grants (SN-8 / the BLOCKER-#5 rule). Returns
 * the server-derived run handles so the caller routes to the live run.
 */

import type { KxClientBase } from "@kortecx/sdk/web";
import { useMutation } from "@tanstack/react-query";
import { useConnection } from "./connection-context";

/** The wire request shape — derived from the SDK so the UI never imports SDK
 *  internals (`@bufbuild/protobuf` / the message schema). */
export type SubmitWorkflowVars = Parameters<KxClientBase["submitWorkflow"]>[0];

export interface SubmittedWorkflow {
  instanceId: string;
  recipeFingerprint: string;
}

export function useSubmitWorkflow() {
  const { client } = useConnection();
  return useMutation<SubmittedWorkflow, unknown, SubmitWorkflowVars>({
    mutationFn: async (request) => {
      if (!client) {
        throw new Error("not connected");
      }
      // No `wait` ⇒ resolves to a `Run` handle (V2a), whose ids are already hex.
      const run = await client.submitWorkflow(request);
      // A `Run` carries `recipeFingerprint` (a committed `Result` does not) —
      // discriminate on it to narrow the union.
      if (!("recipeFingerprint" in run)) {
        throw new Error("unexpected submitWorkflow result");
      }
      return {
        instanceId: run.instanceId,
        recipeFingerprint: run.recipeFingerprint,
      };
    },
  });
}
