/**
 * The durable-Workflow hooks — the read-only catalog (`ListWorkflows`),
 * one Workflow's stored envelope (`GetWorkflow`), and the save / run / delete
 * mutations (`SaveWorkflow` / `RunWorkflow` / `DeleteWorkflow`).
 *
 * A Workflow is a durable, reusable `kortecx.workflow/v1` envelope (a portable
 * blueprint with the App envelope's by-reference references + steering config).
 * `workflowRef` is SERVER-derived; Workflows are caller-scoped (a not-found /
 * not-owned Workflow is uniform — no cross-party existence oracle). The envelope
 * carries NO authority — `runWorkflow` re-compiles the blueprint SERVER-side and
 * re-resolves every warrant from the caller's grants (the RunApp posture).
 * Degrades to a not-wired empty state on an old gateway.
 */

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

/** The `kortecx.workflow/v1` envelope schema tag (mirrors kx-app's WORKFLOW_SCHEMA). */
export const WORKFLOW_SCHEMA = "kortecx.workflow/v1";

/** One row of the caller's Workflow catalog (`ListWorkflows`). */
export interface WorkflowSummary {
  readonly handle: string;
  readonly workflowRef: string;
  readonly name: string;
  readonly version: string;
  readonly description: string;
  readonly tags: string[];
  readonly stepCount: number;
  readonly delivers: string;
  /** Caller-stated lifecycle: `""` (active) or `"draft"`. */
  readonly lifecycle: string;
}

/** One stored Workflow (`GetWorkflow`): the parsed canonical envelope + identity. */
export interface StoredWorkflow {
  readonly envelope: unknown;
  /** The handle-free portable identity (hex). */
  readonly workflowDigest: string;
  readonly lifecycle: string;
  readonly stepCount: number;
}

export function useWorkflows() {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.workflows(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<WorkflowSummary[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listWorkflows();
    },
  });
  return {
    workflows: q.data ?? [],
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    isError: q.isError,
    error: q.error,
    refetch: q.refetch,
  };
}

export function useWorkflow(handle: string | null) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.workflow(endpoint, handle ?? ""),
    enabled: status === "connected" && client !== null && handle !== null,
    queryFn: async (): Promise<StoredWorkflow | null> => {
      if (!client || handle === null) {
        throw new Error("not connected");
      }
      return client.getWorkflow(handle);
    },
  });
}

/**
 * Save (upsert) a Workflow envelope (`SaveWorkflow`). `workflowRef` is
 * SERVER-derived; the envelope carries NO authority (the run re-resolves every
 * warrant). `lifecycle` is caller-stated per save: `""` (active, the default)
 * or `"draft"` — the save IS the authoring act, there is no scaffold loop. On
 * success the Workflow + catalog + its definition-history caches are
 * invalidated (every save records a branch version at the workflow handle).
 */
export function useSaveWorkflow() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<
    { workflowRef: string; handle: string; deduplicated: boolean },
    unknown,
    { handle: string; envelope: unknown; lifecycle?: "" | "draft" }
  >({
    mutationFn: async ({ handle, envelope, lifecycle }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.saveWorkflow(envelope, { handle, lifecycle: lifecycle ?? "" });
    },
    onSuccess: (_res, { handle }) => {
      void qc.invalidateQueries({ queryKey: queryKeys.workflow(endpoint, handle) });
      void qc.invalidateQueries({ queryKey: queryKeys.workflows(endpoint) });
      void qc.invalidateQueries({ queryKey: queryKeys.branchVersions(endpoint, handle) });
    },
  });
}

export interface RunWorkflowResult {
  readonly instanceId: string;
  /** `RunHandle.react_chain_salt` (hex) — empty for any shape without exactly one
   *  tool-granted agentic step (the server's honest answer, never invented here). */
  readonly reactChainSalt: string;
  /** `RunHandle.terminal_mote_id` (hex) — the sink Mote, populated for EVERY shape;
   *  what makes the run view scopable. Reduce the pair with `runAnchor()`. */
  readonly terminalMoteId: string;
}

export function useRunWorkflow() {
  const { client } = useConnection();
  return useMutation<
    RunWorkflowResult,
    unknown,
    { handle: string; args?: Record<string, string>; requireApproval?: boolean }
  >({
    mutationFn: async ({ handle, args, requireApproval }) => {
      if (!client) {
        throw new Error("not connected");
      }
      // No `wait` ⇒ a Run handle (its ids are already hex) — route to the live run.
      const run = await client.runWorkflow(handle, { args, requireApproval });
      if (!("recipeFingerprint" in run)) {
        throw new Error("unexpected runWorkflow result");
      }
      return {
        instanceId: run.instanceId,
        reactChainSalt: run.reactChainSalt,
        terminalMoteId: run.terminalMoteId,
      };
    },
  });
}

/** What `DeleteWorkflow` reports it actually did (row → triggers → lock → branch
 *  binding; blobs and recorded HISTORY stay, so delete + restore recreates). */
export interface DeleteWorkflowResult {
  readonly removed: boolean;
  readonly branchUnbound: boolean;
  readonly lockCleared: boolean;
  readonly triggersRemoved: number;
}

export function useDeleteWorkflow() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<DeleteWorkflowResult, unknown, { handle: string }>({
    mutationFn: async ({ handle }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.deleteWorkflow(handle);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.workflows(endpoint) });
    },
  });
}
