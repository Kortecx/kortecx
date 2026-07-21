/**
 * The navbar approvals DRAWER — a right-side panel over the HITL pre-action approvals
 * queue (`ListPendingApprovals` / `GrantApproval` / `DenyApproval`). Opened from the
 * navbar bell; renders each staged action as a card with Grant / Deny. Reuses the exact
 * honest states from the old in-Apps inbox (not-wired / error / loading / empty) so the
 * approvals invariants (no false badge, honest empty) carry over. A grant releases the
 * staged action once; a deny dead-letters the chain (the server derives `requestId`).
 */

import { Link } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { useApprovalsDrawer } from "../../app/approvals-context";
import {
  type ApprovalDecisionArgs,
  useDenyApproval,
  useGrantApproval,
  useListPendingApprovals,
} from "../../kx/use-approvals";
import { shortHex } from "../../lib/format";
import { memberMoteSearch } from "../../lib/run-anchor";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";

export function ApprovalsDrawer() {
  const { open, close } = useApprovalsDrawer();
  const inbox = useListPendingApprovals();
  const grant = useGrantApproval();
  const deny = useDenyApproval();
  const [acting, setActing] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    function onKey(e: KeyboardEvent): void {
      if (e.key === "Escape") {
        close();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, close]);

  if (!open) {
    return null;
  }

  function decide(mut: ReturnType<typeof useGrantApproval>, args: ApprovalDecisionArgs): void {
    setActing(args.requestId);
    mut.mutate(args, { onSettled: () => setActing(null) });
  }

  return createPortal(
    <>
      <button
        type="button"
        className="node-drawer__scrim node-drawer__scrim--overlay"
        aria-label="Close approvals"
        onClick={close}
      />
      <m.aside
        className="node-drawer node-drawer--overlay"
        data-testid="approvals-drawer"
        // biome-ignore lint/a11y/useSemanticElements: non-modal side panel; dialog semantics via role+aria-label (mirrors AppRunDrawer).
        role="dialog"
        aria-label="Approvals"
        initial={{ x: 24, opacity: 0 }}
        animate={{ x: 0, opacity: 1 }}
        transition={{ type: "spring", stiffness: 420, damping: 34 }}
      >
        <div className="node-drawer__head">
          <h3>Approvals{inbox.count > 0 ? ` (${inbox.count})` : ""}</h3>
          <button type="button" className="linkbtn" onClick={close} aria-label="Close">
            ✕
          </button>
        </div>
        <div className="node-drawer__section">
          <p className="muted">World-mutating actions withheld pending your decision (HITL).</p>
          {inbox.notWired ? (
            <p className="muted" data-testid="approvals-not-wired">
              Not wired on this gateway — pre-action approvals need a serve built with the approval
              admin.
            </p>
          ) : inbox.error ? (
            <ErrorNotice error={inbox.error} />
          ) : inbox.isLoading ? (
            <EmptyState title="Loading approvals…" />
          ) : inbox.approvals.length === 0 ? (
            <EmptyState
              title="No actions awaiting approval"
              detail="A pending item appears when a run's irreversible action is staged under a require-approval posture. Grant to fire it once; deny to dead-letter the chain."
            />
          ) : (
            <div className="approval-cards">
              {inbox.approvals.map((ap) => {
                const busy = acting === ap.requestId;
                return (
                  <div key={ap.requestId} className="approval-card" data-testid="approval-row">
                    <code className="mono">
                      {ap.toolId}@{ap.toolVersion}
                    </code>
                    <p>{ap.intent || "—"}</p>
                    {ap.instanceId ? (
                      <Link
                        to="/workflows/$instanceId"
                        params={{ instanceId: ap.instanceId }}
                        // The pending Mote is a member of the run's component, and the
                        // scope is a connected-component walk — so it anchors the view
                        // just as well as the sink would. Without it, opening a run from
                        // an approval showed the whole shared journal.
                        search={memberMoteSearch(ap.moteId)}
                        className="linkbtn mono"
                        title="Open this run"
                      >
                        {shortHex(ap.instanceId)}
                      </Link>
                    ) : null}
                    <div className="approval-actions">
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
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </m.aside>
    </>,
    document.body,
  );
}
