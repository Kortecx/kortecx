/**
 * Start an agentic TOOL turn: submit a single-MODEL-step workflow carrying the
 * picked tool contract, and return the run's hex instance id + the react chain salt
 * (no wait) so the caller scopes the answer to THIS run's ReAct chain (exactly-once),
 * never a stale committed Mote. Mirrors {@link useInvoke} for the tool-attached chat
 * turn (a plain / agent-mode turn still rides Invoke).
 */

import type { KxClientBase } from "@kortecx/sdk/web";
import { useMutation } from "@tanstack/react-query";
import { useConnection } from "./connection-context";

/** The wire request shape — derived from the SDK so the UI never imports SDK
 *  internals (`@bufbuild/protobuf` / the message schema). */
export type AgentTurnVars = Parameters<KxClientBase["submitWorkflow"]>[0];

export interface StartedAgentTurn {
  instanceId: string;
  /** The per-run react chain key that scopes ListReactTurns to THIS run's chain
   *  ("" only when the server did not tag the step agentic). */
  reactChainSalt: string;
}

export function useSubmitAgentTurn() {
  const { client } = useConnection();
  return useMutation<StartedAgentTurn, unknown, AgentTurnVars>({
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
      return { instanceId: run.instanceId, reactChainSalt: run.reactChainSalt };
    },
  });
}
