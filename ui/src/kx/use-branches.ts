/**
 * The D155 branch hooks — the inventory view (`ListBranches`) plus the
 * create / snapshot / delete mutations (`CreateBranch` / `SnapshotInto` /
 * `DeleteBranch`).
 *
 * A branch is a named, content-addressed `{path → ContentRef}` manifest over
 * operator-approved host files: `snapshot` reads confined host files (under
 * `KX_SERVE_FS_ROOT`, default-OFF) INTO the content store; the agent loop edits
 * them in-CAS (the host is never written in this phase). SN-8: `branchRef` is
 * SERVER-derived; branches are caller-scoped (a not-found / not-owned branch is
 * uniform). Degrades to a not-wired empty state on a gateway without the branch
 * store (UNIMPLEMENTED).
 */

import type { Branch } from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export function useBranches() {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.branches(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<Branch[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listBranches();
    },
  });
  return {
    branches: q.data ?? [],
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    isError: q.isError,
    error: q.error,
    refetch: q.refetch,
  };
}

export interface CreateBranchVars {
  readonly handle: string;
  readonly parent?: string;
  readonly description?: string;
}

export function useCreateBranch() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async ({ handle, parent, description }: CreateBranchVars) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.createBranch(handle, { parent, description });
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.branches(endpoint) });
    },
  });
}

export interface SnapshotVars {
  readonly handle: string;
  readonly paths: readonly string[];
  readonly parent?: string;
  readonly description?: string;
}

export function useSnapshotInto() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async ({ handle, paths, parent, description }: SnapshotVars) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.snapshotInto(handle, paths, { parent, description });
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.branches(endpoint) });
    },
  });
}

export function useDeleteBranch() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<boolean, unknown, { handle: string }>({
    mutationFn: async ({ handle }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.deleteBranch(handle);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.branches(endpoint) });
    },
  });
}

export interface EditBranchVars {
  readonly handle: string;
  readonly path: string;
  readonly instruction: string;
  /**
   * item6: sibling paths in the same branch whose current bodies ride along as
   * read-only context, so a multi-artifact edit driven by one high-level instruction
   * stays coherent across files. Ignored by the one-shot {@link useEditBranch}.
   */
  readonly contextPaths?: readonly string[];
}

/**
 * D155 Phase-3: agentically edit a branch file IN-CAS in one shot. Runs the
 * `kx/recipes/react-edit` loop (the file's current body attached as a context
 * ref; the model rewrites it per `instruction`) and advances the manifest to the
 * new content ref. The host is NEVER written. The mutation can take a while
 * (model inference), so the form shows a pending state.
 */
export function useEditBranch() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<unknown, unknown, EditBranchVars>({
    mutationFn: async ({ handle, path, instruction }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.editBranch(handle, path, instruction);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.branches(endpoint) });
    },
  });
}

export interface EditProposalResult {
  readonly resultRef: string;
  readonly proposedText: string;
  readonly currentText: string;
}

/**
 * POC-5d: the PROPOSE half of the agentic edit (the review/diff gate) — runs the
 * `kx/recipes/react-edit` loop and returns the model's proposed new body + the
 * current body WITHOUT advancing the branch. The user reviews the diff and then
 * either approves ({@link useAdvanceBranch} with `resultRef`) or rejects (discards —
 * the proposed blob is a harmless content-addressed orphan). Closes
 * `T-AGENTIC-EDIT-REVIEW-GATE`. No invalidation here (nothing changed yet).
 *
 * item6: `contextPaths` attaches sibling files as coherence context so the caller can
 * drive a MULTI-artifact modify (one propose per file, one high-level instruction).
 */
export function useEditBranchPropose() {
  const { client } = useConnection();
  return useMutation<EditProposalResult, unknown, EditBranchVars>({
    mutationFn: async ({ handle, path, instruction, contextPaths }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.editBranchPropose(handle, path, instruction, { contextPaths });
    },
  });
}

export interface AdvanceBranchVars {
  readonly handle: string;
  readonly path: string;
  readonly contentRef: string;
}

/**
 * POC-5d: APPROVE a proposed edit — re-point the manifest path to the committed
 * `contentRef` (`AdvanceBranch`). A LOCKED App is refused at the server chokepoint.
 * Invalidates the App branch manifest so the new body is re-pulled.
 */
export function useAdvanceBranch() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<unknown, unknown, AdvanceBranchVars>({
    mutationFn: async ({ handle, path, contentRef }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.advanceBranch(handle, path, contentRef);
    },
    onSuccess: (_res, { handle }) => {
      void qc.invalidateQueries({ queryKey: queryKeys.branches(endpoint) });
      void qc.invalidateQueries({ queryKey: queryKeys.appBranch(endpoint, handle) });
    },
  });
}
