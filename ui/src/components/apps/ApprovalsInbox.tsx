import { Link } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useState } from "react";
import { stagger } from "../../app/motion";
import {
  type ApprovalDecisionArgs,
  useDenyApproval,
  useGrantApproval,
  useListPendingApprovals,
} from "../../kx/use-approvals";
import { shortHex } from "../../lib/format";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { GlowCard } from "../ds/GlowCard";
import { MetricCard } from "../metrics/MetricCard";

/**
 * The HITL pre-action approvals INBOX — the govern surface over world-mutating actions
 * a live autonomous run has STAGED and withheld pending your decision
 * (`ListPendingApprovals` / `GrantApproval` / `DenyApproval`). A grant releases the
 * staged action to fire exactly once; a deny dead-letters the chain. The server derives
 * the `requestId` — you decide, never mint authority. Polled (approvals are not on the
 * event stream). A single cross-App queue; degrades to an honest not-wired note without
 * the approval admin.
 */
export function ApprovalsInbox() {
  const inbox = useListPendingApprovals();
  const grant = useGrantApproval();
  const deny = useDenyApproval();
  // Track the requestId mid-decision so only that row's controls disable.
  const [acting, setActing] = useState<string | null>(null);

  function decide(mut: ReturnType<typeof useGrantApproval>, args: ApprovalDecisionArgs): void {
    setActing(args.requestId);
    mut.mutate(args, { onSettled: () => setActing(null) });
  }

  return (
    <GlowCard hover={false} className="monitor-panel" data-testid="apps-approvals">
      <div className="monitor-panel__head">
        <h2>Approvals</h2>
        <span className="muted">world-mutating actions withheld pending your decision (HITL)</span>
      </div>
      {inbox.notWired ? (
        <p className="muted" data-testid="approvals-not-wired">
          Not wired on this gateway — pre-action approvals need a serve built with the approval
          admin (an autonomous run requests a decision only under a require-approval posture).
        </p>
      ) : inbox.error ? (
        <ErrorNotice error={inbox.error} />
      ) : inbox.isLoading ? (
        <EmptyState title="Loading approvals…" />
      ) : inbox.approvals.length === 0 ? (
        <EmptyState
          title="No actions awaiting approval"
          detail="A pending item appears when a run's irreversible action (an email send, a channel post, a DB write) is staged under a require-approval posture. Grant to fire it once; deny to dead-letter the chain."
        />
      ) : (
        <>
          <m.div
            className="metrics-grid"
            variants={stagger()}
            initial="hidden"
            animate="show"
            data-testid="approvals-kpis"
          >
            <MetricCard
              label="Awaiting"
              value={inbox.count}
              tone="scheduled"
              sub="pending decisions"
            />
          </m.div>
          <table className="trail-table" data-testid="approvals-table">
            <thead>
              <tr>
                <th>Tool</th>
                <th>Intent</th>
                <th>Run</th>
                <th>Requested</th>
                <th>Deadline</th>
                <th>Decision</th>
              </tr>
            </thead>
            <tbody>
              {inbox.approvals.map((ap) => {
                const busy = acting === ap.requestId;
                return (
                  <tr key={ap.requestId} data-testid="approval-row">
                    <td className="mono">
                      {ap.toolId}@{ap.toolVersion}
                    </td>
                    <td>{ap.intent || "—"}</td>
                    <td className="mono">
                      {ap.instanceId ? (
                        <Link
                          to="/workflows/$instanceId"
                          params={{ instanceId: ap.instanceId }}
                          className="linkbtn mono"
                          title="Open this run"
                        >
                          {shortHex(ap.instanceId)}
                        </Link>
                      ) : (
                        "—"
                      )}
                    </td>
                    <td className="muted">
                      {ap.createdUnixMs > 0 ? new Date(ap.createdUnixMs).toLocaleTimeString() : "—"}
                    </td>
                    <td className="muted">
                      {ap.deadlineUnixMs > 0
                        ? new Date(ap.deadlineUnixMs).toLocaleTimeString()
                        : "—"}
                    </td>
                    <td className="approval-actions">
                      <button
                        type="button"
                        className="btn-primary"
                        data-testid="approval-grant-btn"
                        disabled={busy}
                        onClick={() => decide(grant, { requestId: ap.requestId })}
                      >
                        {busy && grant.isPending ? "…" : "Grant"}
                      </button>
                      <button
                        type="button"
                        className="btn-ghost"
                        data-testid="approval-deny-btn"
                        disabled={busy}
                        onClick={() => decide(deny, { requestId: ap.requestId })}
                      >
                        {busy && deny.isPending ? "…" : "Deny"}
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {grant.isError || deny.isError ? (
            <p className="field-error" data-testid="approval-decision-error" role="alert">
              The last decision failed — it may have already resolved or expired. The list refreshes
              automatically.
            </p>
          ) : null}
        </>
      )}
    </GlowCard>
  );
}
