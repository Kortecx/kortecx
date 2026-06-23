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

import type { ContextBundle, ContextItemInput, PutContextBundleResult } from "@kortecx/sdk/web";
import { KxError, KxFailedPrecondition } from "@kortecx/sdk/web";
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

// ----- POC-2 context-edit (item view/edit/rename/remove + description) --------
// CAS is IMMUTABLE: an item edit uploads NEW bytes (a new ref) then re-upserts the
// bundle with that item re-pointed — a pure-client compose over existing RPCs, so
// digest-invariant by construction. Every mutation carries an optional
// `expectBundleRef` (the manifest the user was viewing): the content-addressed
// `bundleRef` is a free compare-and-swap token, so a concurrent change is REFUSED
// (`KxFailedPrecondition`) rather than silently clobbered.

/** Re-read `manifest` + run the stale-base guard. A mismatch ⇒ the bundle changed
 *  under the editor — fail closed, never a silent last-writer-wins overwrite. */
function assertFresh(
  manifest: ContextBundle | null,
  handle: string,
  expectBundleRef: string | undefined,
): ContextBundle {
  if (manifest === null) {
    throw new KxError(`context bundle '${handle}' not found`);
  }
  if (expectBundleRef !== undefined && manifest.bundleRef !== expectBundleRef) {
    throw new KxFailedPrecondition(
      `context bundle '${handle}' changed since you opened it — reload and re-apply your change`,
    );
  }
  return manifest;
}

export interface EditContextItemVars {
  readonly handle: string;
  readonly itemIndex: number;
  readonly text: string;
  readonly mediaType?: string;
  readonly expectBundleRef?: string;
}

/** Replace one item's body (the headline edit). Delegates to the SDK's
 *  `editContextItem` (which uploads + re-upserts + runs the stale-base guard). */
export function useEditContextItem() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<PutContextBundleResult, unknown, EditContextItemVars>({
    mutationFn: async ({ handle, itemIndex, text, mediaType, expectBundleRef }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.editContextItem(handle, itemIndex, new TextEncoder().encode(text), {
        mediaType,
        expectBundleRef,
      });
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.contextBundles(endpoint) });
    },
  });
}

export interface RemoveContextItemVars {
  readonly handle: string;
  readonly itemIndex: number;
  readonly expectBundleRef?: string;
}

/** Drop one item (re-upsert the remainder). Refuses to empty the bundle. */
export function useRemoveContextItem() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<PutContextBundleResult, unknown, RemoveContextItemVars>({
    mutationFn: async ({ handle, itemIndex, expectBundleRef }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.removeContextItem(handle, itemIndex, { expectBundleRef });
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.contextBundles(endpoint) });
    },
  });
}

export interface RenameContextItemVars {
  readonly handle: string;
  readonly itemIndex: number;
  readonly newName: string;
  readonly expectBundleRef?: string;
}

/** Re-label one item (no body change) — a guarded re-upsert with the new name. */
export function useRenameContextItem() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<PutContextBundleResult, unknown, RenameContextItemVars>({
    mutationFn: async ({ handle, itemIndex, newName, expectBundleRef }) => {
      if (!client) {
        throw new Error("not connected");
      }
      const manifest = assertFresh(await client.getContextBundle(handle), handle, expectBundleRef);
      const items: ContextItemInput[] = manifest.items.map((it, i) => ({
        name: i === itemIndex ? newName : it.name,
        contentRef: it.contentRef,
        mediaType: it.mediaType,
      }));
      return client.putContextBundle(handle, items, { description: manifest.description });
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.contextBundles(endpoint) });
    },
  });
}

export interface EditBundleDescriptionVars {
  readonly handle: string;
  readonly description: string;
  readonly expectBundleRef?: string;
}

/** Re-set a bundle's advisory description — a guarded re-upsert, items unchanged. */
export function useEditBundleDescription() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<PutContextBundleResult, unknown, EditBundleDescriptionVars>({
    mutationFn: async ({ handle, description, expectBundleRef }) => {
      if (!client) {
        throw new Error("not connected");
      }
      const manifest = assertFresh(await client.getContextBundle(handle), handle, expectBundleRef);
      const items: ContextItemInput[] = manifest.items.map((it) => ({
        name: it.name,
        contentRef: it.contentRef,
        mediaType: it.mediaType,
      }));
      return client.putContextBundle(handle, items, { description });
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.contextBundles(endpoint) });
    },
  });
}
