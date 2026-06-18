/**
 * The context-bundle hooks (PR-7) — the inventory view (`ListContextBundles`)
 * plus the author/delete mutations (`PutContextBundle` / `DeleteContextBundle`).
 *
 * A context bundle is named, content-addressed grounding the caller attaches to a
 * run (`invoke(handle, args, { context: [handle] })`); the server resolves it to
 * its content-refs and folds them into the entry Mote's identity-bearing config,
 * so a different attached context ⇒ a different run. SN-8: `bundleRef` is
 * SERVER-derived; bundles are caller-scoped (a not-found / not-owned bundle is
 * uniform — no cross-party existence oracle). Degrades to a not-wired empty state
 * on a gateway without the bundle store (UNIMPLEMENTED).
 */

import type { ContextBundle, ContextItemInput } from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export function useContextBundles() {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.contextBundles(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<ContextBundle[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listContextBundles();
    },
  });
  return {
    bundles: q.data ?? [],
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    isError: q.isError,
    error: q.error,
    refetch: q.refetch,
  };
}

export interface PutContextBundleVars {
  readonly handle: string;
  readonly items: readonly ContextItemInput[];
  readonly description?: string;
}

export function usePutContextBundle() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async ({ handle, items, description }: PutContextBundleVars) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.putContextBundle(handle, items, { description });
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.contextBundles(endpoint) });
    },
  });
}

export function useDeleteContextBundle() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<boolean, unknown, { handle: string }>({
    mutationFn: async ({ handle }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.deleteContextBundle(handle);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.contextBundles(endpoint) });
    },
  });
}
