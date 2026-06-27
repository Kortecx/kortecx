/**
 * The trigger admin hooks (D113 / D170.b): the local trigger registry — list
 * (`ListTriggers`), register (`RegisterTrigger`), dry-run validate (`TestTrigger`),
 * fire the inbound EVENT (`SubmitTrigger`), and remove (`DeregisterTrigger`). A
 * trigger binds an inbound event (a webhook POST, a cron interval, or a bare
 * `SubmitTrigger` RPC) to a recipe handle the event Invokes.
 *
 * SN-8: `triggerId`/`instanceId` are server-derived; the auth secret is referenced
 * by NAME only (never the value, D81 — a `ListTriggers` row carries
 * `authSecretPresent`, never the secret itself). The minimal-local single-user
 * trigger; the hosted multi-tenant trigger gateway at scale is CLOUD (GR19).
 * Degrades to a not-wired empty state on a gateway without triggers (UNIMPLEMENTED).
 */

import type {
  RegisterTriggerInput,
  RegisterTriggerResult,
  SubmitTriggerResult,
  TestTriggerResult,
  TriggerRow,
} from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

/** A reasonable first page of triggers (the local registry is single-user / small). */
const TRIGGERS_PAGE = 200;

export function useListTriggers() {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.triggers(endpoint, TRIGGERS_PAGE),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<readonly TriggerRow[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      const page = await client.triggers.list({ limit: TRIGGERS_PAGE });
      return page.triggers;
    },
  });
  return {
    triggers: q.data ?? [],
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    isError: q.isError,
    error: q.error,
    refetch: q.refetch,
  };
}

export function useRegisterTrigger() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<RegisterTriggerResult, unknown, RegisterTriggerInput>({
    mutationFn: async (input) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.triggers.add(input);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.triggers(endpoint, TRIGGERS_PAGE) });
    },
  });
}

/** Arguments for a trigger dry-run (`TestTrigger`) — validates the binding, fires nothing. */
export interface TestTriggerArgs {
  readonly name: string;
  /** Optional event-body JSON to bind-check (empty ⇒ `{}`). */
  readonly payload?: string;
}

export function useTestTrigger() {
  const { client } = useConnection();
  return useMutation<TestTriggerResult, unknown, TestTriggerArgs>({
    mutationFn: async ({ name, payload }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.triggers.test(name, payload);
    },
  });
}

/** Arguments for firing a trigger (`SubmitTrigger`) — the inbound EVENT verb. */
export interface FireTriggerArgs {
  readonly name: string;
  /** Optional event-body JSON (empty ⇒ `{}`). */
  readonly payload?: string;
  /** Optional event-level idempotency key (empty ⇒ server-derived from the payload). */
  readonly idempotencyKey?: string;
}

export function useFireTrigger() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<SubmitTriggerResult, unknown, FireTriggerArgs>({
    mutationFn: async ({ name, payload, idempotencyKey }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.triggers.fire(name, payload, idempotencyKey);
    },
    onSuccess: () => {
      // Firing updates the row's last-fired clock — refresh the govern list.
      void qc.invalidateQueries({ queryKey: queryKeys.triggers(endpoint, TRIGGERS_PAGE) });
    },
  });
}

export function useDeregisterTrigger() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<boolean, unknown, string>({
    mutationFn: async (name) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.triggers.remove(name);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.triggers(endpoint, TRIGGERS_PAGE) });
    },
  });
}
