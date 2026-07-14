/**
 * NL workflow authoring — propose-then-confirm (D209.3 / SN-8).
 *
 * `ProposeWorkflow` turns a natural-language goal into a PROPOSED multi-step DAG: the
 * served model plans, the gateway decodes + compiles the plan through the vetted planner
 * (the model names only role + intent + edges; every capability axis is server-vetted), and
 * returns it for the author to preview. It VALIDATES ONLY — nothing runs until the author
 * applies the proposed steps to the canvas and saves/submits. `{ proposed: false }` carries
 * an honest rejection (no served model, an inadmissible plan) surfaced verbatim.
 */

import type { WorkflowProposal } from "@kortecx/sdk/web";
import { useMutation } from "@tanstack/react-query";
import { useConnection } from "./connection-context";

/** Propose a multi-step workflow DAG from a natural-language goal. The caller previews the
 *  returned {@link WorkflowProposal} and applies it to the canvas to confirm. */
export function useProposeWorkflow() {
  const { client } = useConnection();
  return useMutation<WorkflowProposal, unknown, string>({
    mutationFn: async (goal) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.proposeWorkflow(goal.trim());
    },
  });
}
