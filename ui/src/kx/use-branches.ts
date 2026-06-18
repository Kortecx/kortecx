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
}

/**
 * D155 Phase-3: agentically edit a branch file IN-CAS. Runs the
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
