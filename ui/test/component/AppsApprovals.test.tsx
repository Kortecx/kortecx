/**
 * The cross-App HITL approvals inbox (relocated into Apps). Pins the row + grant/deny
 * wiring, the count KPI, the actionable empty state, and the honest not-wired degrade.
 * Rows are UNATTRIBUTED (`instanceId: ""` → the "—" run cell) so it renders without a
 * router; the attributed run-Link + the live grant flow are covered by the Playwright
 * spec. The FinOps spend column is intentionally absent (cost is not an OSS surface).
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ApprovalsInbox } from "../../src/components/apps/ApprovalsInbox";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

function unimplemented(): never {
  throw Object.assign(new Error("not wired"), { code: "unimplemented" });
}

/** A pending-approval row shaped like the SDK's `PendingApprovalRow` (plain fields;
 *  the component reads them directly). Unattributed to stay router-free. */
function pending(requestId: string, toolId = "slack/post_message") {
  return {
    requestId,
    instanceId: "",
    moteId: "bb".repeat(16),
    toolId,
    toolVersion: "1",
    intent: `fire ${toolId}`,
    deadlineUnixMs: 0,
    createdUnixMs: 1_700_000_000_000,
  };
}

describe("ApprovalsInbox (HITL approvals, relocated into Apps)", () => {
  it("renders pending approvals with Grant/Deny controls + a count KPI", async () => {
    const mock = makeMockClient({
      approvalsListPending: async () => ({
        approvals: [pending("cc".repeat(16)), pending("dd".repeat(16), "notion/create_page")],
      }),
    });
    render(<ApprovalsInbox />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getAllByTestId("approval-row")).toHaveLength(2));
    expect(screen.getByTestId("apps-approvals")).toBeInTheDocument();
    expect(screen.getByText("slack/post_message@1")).toBeInTheDocument();
    expect(screen.getByText("notion/create_page@1")).toBeInTheDocument();
    expect(screen.getAllByTestId("approval-grant-btn")).toHaveLength(2);
    expect(screen.getAllByTestId("approval-deny-btn")).toHaveLength(2);
    expect(screen.getByTestId("approvals-kpis")).toHaveTextContent("2");
    // No FinOps spend column (cost is not an OSS surface).
    expect(screen.queryByText("Spend")).toBeNull();
    expect(screen.queryByTestId("approval-cost")).toBeNull();
  });

  it("Grant fires GrantApproval with the server-derived requestId", async () => {
    const rid = "cc".repeat(16);
    const mock = makeMockClient({
      approvalsListPending: async () => ({ approvals: [pending(rid)] }),
    });
    render(<ApprovalsInbox />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getByTestId("approval-grant-btn")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("approval-grant-btn"));
    await waitFor(() => expect(mock.approvalsGrant).toHaveBeenCalledWith(rid, ""));
  });

  it("Deny fires DenyApproval with the requestId", async () => {
    const rid = "dd".repeat(16);
    const mock = makeMockClient({
      approvalsListPending: async () => ({ approvals: [pending(rid)] }),
    });
    render(<ApprovalsInbox />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getByTestId("approval-deny-btn")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("approval-deny-btn"));
    await waitFor(() => expect(mock.approvalsDeny).toHaveBeenCalledWith(rid, ""));
  });

  it("shows the actionable empty state when nothing is awaiting a decision", async () => {
    const mock = makeMockClient(); // default listPending → { approvals: [] }
    render(<ApprovalsInbox />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() =>
      expect(screen.getByText(/No actions awaiting approval/i)).toBeInTheDocument(),
    );
  });

  it("degrades to the honest not-wired note without the approval admin", async () => {
    const mock = makeMockClient({ approvalsListPending: async () => unimplemented() });
    render(<ApprovalsInbox />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getByTestId("approvals-not-wired")).toBeInTheDocument());
  });
});
