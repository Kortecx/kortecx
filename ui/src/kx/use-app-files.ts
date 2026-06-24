/**
 * POC-5d App project-file hooks — read an App's CoW branch manifest
 * (`GetBranch`) and one file's FULL body (`GetBranchContent`).
 *
 * By convention the App's project branch handle IS the App's own handle
 * (one-App-one-branch), so every read keys on the App handle. The branch is
 * caller-scoped (SN-8): a not-found / not-owned branch resolves to `null` (no
 * existence oracle). File bodies are content-addressed, so the content query
 * caches forever per (endpoint, handle, ref) — enable it only for the selected
 * file so a wide tree never N+1-fetches every body.
 */

import type { AdvanceResult, Branch } from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { decodeContent } from "../lib/content-decode";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

/** The App's project branch manifest (`{path → contentRef}` items). */
export function useAppBranch(handle: string | null) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.appBranch(endpoint, handle ?? ""),
    enabled: status === "connected" && client !== null && handle !== null,
    queryFn: async (): Promise<Branch | null> => {
      if (!client || handle === null) {
        throw new Error("not connected");
      }
      return client.getBranch(handle);
    },
  });
}

export interface AppFileBody {
  /** The decoded UTF-8 text of the file (empty for a missing/binary body). */
  readonly text: string;
  /** True when the path resolved to no body (absent file / not-owned branch). */
  readonly missing: boolean;
}

/**
 * One App project file's body, keyed by its content ref (content-addressed ⇒
 * immutable, cache forever). Pass the `contentRef` from the branch manifest so
 * the cache key is stable across re-fetches and the body is fetched at most once.
 */
export function useAppFileContent(
  handle: string | null,
  path: string | null,
  contentRef: string | null,
  enabled: boolean,
) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.appFileContent(endpoint, handle ?? "", contentRef ?? ""),
    enabled:
      enabled &&
      status === "connected" &&
      client !== null &&
      handle !== null &&
      path !== null &&
      contentRef !== null,
    staleTime: Number.POSITIVE_INFINITY, // content-addressed ⇒ immutable
    retry: false,
    queryFn: async (): Promise<AppFileBody> => {
      if (!client || handle === null || path === null) {
        throw new Error("not connected");
      }
      const bytes = await client.getBranchContent(handle, path);
      if (bytes === null) {
        return { text: "", missing: true };
      }
      return { text: decodeContent(bytes).text, missing: false };
    },
  });
}

export interface SaveFileVars {
  readonly handle: string;
  readonly path: string;
  readonly text: string;
}

/**
 * POC-5d: directly save a file's edited body IN-CAS — `PutContent` (the typed bytes,
 * server-derived ref, SN-8) → `AdvanceBranch` (re-point the manifest path). The host
 * is NEVER written. A LOCKED App is refused at the AdvanceBranch chokepoint
 * (`FAILED_PRECONDITION` + `LOCKED_BRANCH`); the UI also disables the editor when
 * locked (GR15). On success the branch manifest is invalidated so the new ref/body is
 * re-pulled (re-base the editor draft — never an optimistic clobber).
 */
export function useSaveFile() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<AdvanceResult, unknown, SaveFileVars>({
    mutationFn: async ({ handle, path, text }) => {
      if (!client) {
        throw new Error("not connected");
      }
      const put = await client.putContent(new TextEncoder().encode(text), { filename: path });
      return client.advanceBranch(handle, path, put.contentRef);
    },
    onSuccess: (_res, { handle }) => {
      void qc.invalidateQueries({ queryKey: queryKeys.appBranch(endpoint, handle) });
    },
  });
}
