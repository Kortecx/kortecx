/**
 * The POC-4 App-catalog hooks — the read-only inventory (`ListApps`), one App's
 * full envelope (`GetApp`), and the `runApp` mutation (client-compose over
 * `GetApp` → `submitWorkflow`).
 *
 * An App is a durable, reusable `kortecx.app/v1` envelope (a portable blueprint
 * wrapped with by-reference references + a 4-axis steering config). SN-8: `appRef`
 * is SERVER-derived; Apps are caller-scoped (a not-found / not-owned App is uniform
 * — no cross-party existence oracle). The envelope carries NO authority — `runApp`
 * re-compiles the blueprint and the server re-resolves every warrant from the
 * caller's grants. Degrades to a not-wired empty state on an old gateway.
 */

import type { AppSummary, StoredApp } from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export function useApps() {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.apps(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<AppSummary[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listApps();
    },
  });
  return {
    apps: q.data ?? [],
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    isError: q.isError,
    error: q.error,
    refetch: q.refetch,
  };
}

export function useApp(handle: string | null) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.app(endpoint, handle ?? ""),
    enabled: status === "connected" && client !== null && handle !== null,
    queryFn: async (): Promise<StoredApp | null> => {
      if (!client || handle === null) {
        throw new Error("not connected");
      }
      return client.getApp(handle);
    },
  });
}

export interface RunAppResult {
  readonly instanceId: string;
}

export function useRunApp() {
  const { client } = useConnection();
  return useMutation<
    RunAppResult,
    unknown,
    { handle: string; args?: Record<string, string>; requireApproval?: boolean }
  >({
    mutationFn: async ({ handle, args, requireApproval }) => {
      if (!client) {
        throw new Error("not connected");
      }
      // No `wait` ⇒ a Run handle (its ids are already hex) — route to the live run.
      // POC-5d: `args` (the App's input_schema inputs) fold into the entry model step.
      // `requireApproval` (opt-in) runs the entry step under the per-run HITL gate, so a
      // world-mutating tool call surfaces in the approvals inbox before it fires.
      const run = await client.runApp(handle, { args, requireApproval });
      if (!("recipeFingerprint" in run)) {
        throw new Error("unexpected runApp result");
      }
      return { instanceId: run.instanceId };
    },
  });
}

/**
 * POC-5d: persist an edited App envelope (`SaveApp`) — the structure edit the
 * Lineage editor commits. SN-8: `appRef` is SERVER-derived; the envelope carries NO
 * authority (the run re-resolves every warrant). A LOCKED App refuses the save at the
 * server with `FAILED_PRECONDITION` + `LOCKED_BRANCH` (the UI also pre-gates on
 * `summary.locked` so the Save control is never shown for a locked App — GR15). On
 * success the App + branch caches are invalidated so the new version shows everywhere.
 */
export function useSaveApp() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<
    { appRef: string; deduplicated: boolean },
    unknown,
    { handle: string; envelope: unknown }
  >({
    mutationFn: async ({ handle, envelope }) => {
      if (!client) {
        throw new Error("not connected");
      }
      const res = await client.saveApp(envelope, { handle });
      return { appRef: res.appRef, deduplicated: res.deduplicated };
    },
    onSuccess: (_res, { handle }) => {
      void qc.invalidateQueries({ queryKey: queryKeys.app(endpoint, handle) });
      void qc.invalidateQueries({ queryKey: queryKeys.apps(endpoint) });
    },
  });
}

/**
 * Export a saved App as a portable `kortecx.appbundle/v1` archive (the envelope PLUS
 * its content-store closure; `withData` includes RAG payloads). The mutation returns
 * the wire string; the caller triggers a browser download. Reuses the SDK client —
 * no new eager weight.
 */
export function useExportAppBundle() {
  const { client } = useConnection();
  return useMutation<string, unknown, { handle: string; withData?: boolean }>({
    mutationFn: async ({ handle, withData }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.exportAppBundle(handle, { withData: withData ?? false });
    },
  });
}

/**
 * Import a `kortecx.appbundle/v1` archive under the caller's OWN principal
 * (fail-closed): PutContent the closure, then SaveApp with a `source_digest` lineage
 * stamp. Connections/secrets never travel — re-register them by name (the App fails
 * closed at run until then). On success the App inventory is invalidated.
 */
export function useImportApp() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<{ handle: string }, unknown, { bundle: string; force?: boolean }>({
    mutationFn: async ({ bundle, force }) => {
      if (!client) {
        throw new Error("not connected");
      }
      const res = await client.importApp(bundle, { force: force ?? false });
      return { handle: res.handle };
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.apps(endpoint) });
    },
  });
}

/**
 * Clone one of the caller's Apps locally under a new name (a frozen copy; content is
 * already resident, so no transfer). Records the source's `app_digest` lineage. On
 * success the inventory is invalidated so the copy appears.
 */
export function useCloneApp() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<{ handle: string }, unknown, { handle: string; newname: string }>({
    mutationFn: async ({ handle, newname }) => {
      if (!client) {
        throw new Error("not connected");
      }
      const res = await client.cloneApp(handle, newname);
      return { handle: res.handle };
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.apps(endpoint) });
    },
  });
}

/** The reserved catalog tag that marks an App as a reusable Template. */
export const TEMPLATE_TAG = "template";

/**
 * Mark / unmark a saved App as a reusable Template — toggles the reserved
 * {@link TEMPLATE_TAG} in the App envelope's `tags` and re-saves (`GetApp` →
 * `SaveApp`; no new RPC). A LOCKED App refuses the save server-side. On success the
 * App + inventory caches are invalidated so the Templates gallery updates.
 */
export function useToggleTemplate() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<{ isTemplate: boolean }, unknown, { handle: string }>({
    mutationFn: async ({ handle }) => {
      if (!client) {
        throw new Error("not connected");
      }
      const app = await client.getApp(handle);
      if (!app) {
        throw new Error("app not found");
      }
      const env = { ...(app.envelope as Record<string, unknown>) };
      const tags = Array.isArray(env.tags) ? (env.tags as string[]) : [];
      const has = tags.includes(TEMPLATE_TAG);
      const nextTags = has ? tags.filter((t) => t !== TEMPLATE_TAG) : [...tags, TEMPLATE_TAG];
      const { tags: _drop, ...rest } = env;
      const nextEnv = nextTags.length > 0 ? { ...rest, tags: nextTags } : rest;
      await client.saveApp(nextEnv, { handle });
      return { isTemplate: !has };
    },
    onSuccess: (_res, { handle }) => {
      void qc.invalidateQueries({ queryKey: queryKeys.app(endpoint, handle) });
      void qc.invalidateQueries({ queryKey: queryKeys.apps(endpoint) });
    },
  });
}
