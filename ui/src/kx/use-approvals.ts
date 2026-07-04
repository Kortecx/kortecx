/**
 * RC6a approvals-inbox hooks (D114) — the console govern surface over pending HITL
 * pre-action approvals: list the withheld world-mutating actions
 * (`ListPendingApprovals`), and GRANT / DENY an operator decision over a
 * server-derived `requestId` (`GrantApproval` / `DenyApproval`). Grant/deny release
 * or reject a STAGED action; they never mint a client warrant (SN-8). The backend
 * (autonomy-safety-gates #267 — journal `KIND_APPROVAL`, the coordinator pause/
 * grant/resume gate, all four RPCs) is fully wired + E2E-tested; this is the UI leg
 * that lets a NON-CLI operator govern an autonomous, trigger-firing App.
 *
 * Approvals are NOT on the global event stream, so the inbox POLLS (the `useAlerts`
 * inbox precedent). Degrades to a not-wired empty state on a gateway without the
 * approval admin (UNIMPLEMENTED).
 */

import type { PendingApprovalRow } from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

/** A generous inbox page — pending approvals are few (one per withheld action). */
const APPROVALS_PAGE = 100;
/** Poll cadence: a pending approval gates a LIVE autonomous run, so refresh briskly. */
const APPROVALS_POLL_MS = 4000;

/** The pending-approvals inbox (`ListPendingApprovals`), polled while connected. */
export function useListPendingApprovals() {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.pendingApprovals(endpoint, APPROVALS_PAGE),
    enabled: status === "connected" && client !== null,
    refetchInterval: APPROVALS_POLL_MS,
    queryFn: async (): Promise<readonly PendingApprovalRow[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      const page = await client.approvals.listPending(APPROVALS_PAGE);
      return page.approvals;
    },
  });
  return {
    approvals: q.data ?? [],
    /** The nav-badge count — pending approvals awaiting an operator decision. */
    count: q.data?.length ?? 0,
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    isError: q.isError,
    error: q.isError && toUiError(q.error).kind !== "not-wired" ? toUiError(q.error) : null,
    refetch: q.refetch,
  };
}

/** An operator approval decision (grant/deny) over a server-derived `requestId`. */
export interface ApprovalDecisionArgs {
  readonly requestId: string;
  /** Optional operator note recorded with the decision. */
  readonly reason?: string;
}

/** Grant a pending approval — release the staged action to fire exactly once. */
export function useGrantApproval() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<boolean, unknown, ApprovalDecisionArgs>({
    mutationFn: async ({ requestId, reason }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.approvals.grant(requestId, reason ?? "");
    },
    onSuccess: () => {
      void qc.invalidateQueries({
        queryKey: queryKeys.pendingApprovals(endpoint, APPROVALS_PAGE),
      });
    },
  });
}

/** Deny a pending approval — reject the staged action (the chain dead-letters). */
export function useDenyApproval() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<boolean, unknown, ApprovalDecisionArgs>({
    mutationFn: async ({ requestId, reason }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.approvals.deny(requestId, reason ?? "");
    },
    onSuccess: () => {
      void qc.invalidateQueries({
        queryKey: queryKeys.pendingApprovals(endpoint, APPROVALS_PAGE),
      });
    },
  });
}
