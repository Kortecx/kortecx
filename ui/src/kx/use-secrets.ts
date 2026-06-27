/**
 * The host secret-store admin hooks (MM-3 / D110): the local OS-keychain secret
 * store — list NAMES (`ListSecretNames`), set/overwrite a value (`PutSecret`), and
 * remove (`DeleteSecret`). A `SecretRef` NAME is what a connection's / trigger's
 * `credential_ref` points at; the VALUE is WRITE-ONLY — it appears ONLY as a
 * `PutSecret` argument and is never returned on any read (D81). `ListSecretNames`
 * surfaces NAMES + audit timestamps only.
 *
 * SN-8: writes are gated loopback-only + an authenticated party server-side; the
 * SDK only *carries* the value to the handler. Degrades to a not-wired empty state
 * on a gateway without the secret store (UNIMPLEMENTED).
 */

import type { SecretNameRow } from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

/** A reasonable first page of secret names (the local store is single-user / small). */
const SECRETS_PAGE = 200;

export function useListSecretNames() {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.secretNames(endpoint, SECRETS_PAGE),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<readonly SecretNameRow[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      const page = await client.secrets.list({ limit: SECRETS_PAGE });
      return page.names;
    },
  });
  return {
    names: q.data ?? [],
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    isError: q.isError,
    error: q.error,
    refetch: q.refetch,
  };
}

/** Arguments for a secret upsert (`PutSecret`). The VALUE is write-only (D81). */
export interface PutSecretArgs {
  readonly name: string;
  readonly value: string;
}

export function usePutSecret() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<boolean, unknown, PutSecretArgs>({
    mutationFn: async ({ name, value }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.secrets.set(name, value);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.secretNames(endpoint, SECRETS_PAGE) });
    },
  });
}

export function useDeleteSecret() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<boolean, unknown, string>({
    mutationFn: async (name) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.secrets.remove(name);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.secretNames(endpoint, SECRETS_PAGE) });
    },
  });
}
