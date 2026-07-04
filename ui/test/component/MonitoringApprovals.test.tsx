/**
 * RC6a — the Monitoring "Approvals" tab: the HITL pre-action approvals inbox. Every
 * D142.3 state is pinned — rows + grant/deny wiring, the empty state, and the honest
 * not-wired degrade — plus the tab fieldset contract. Rows are UNATTRIBUTED
 * (`instanceId: ""` → the "—" run cell) so the section renders without a router; the
 * attributed run-Link + the live grant flow are covered by the approvals Playwright spec.
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ApprovalCost, MonitoringSection } from "../../src/components/sections/MonitoringSection";
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

describe("MonitoringSection — Approvals tab (RC6a)", () => {
  it("renders pending approvals with Grant/Deny controls", async () => {
    const mock = makeMockClient({
      approvalsListPending: async () => ({
        approvals: [pending("cc".repeat(16)), pending("dd".repeat(16), "notion/create_page")],
      }),
    });
    render(<MonitoringSection tab="approvals" />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getAllByTestId("approval-row")).toHaveLength(2));
    expect(screen.getByText("slack/post_message@1")).toBeInTheDocument();
    expect(screen.getByText("notion/create_page@1")).toBeInTheDocument();
    expect(screen.getAllByTestId("approval-grant-btn")).toHaveLength(2);
    expect(screen.getAllByTestId("approval-deny-btn")).toHaveLength(2);
    // The count KPI reflects the inbox size.
    expect(screen.getByTestId("approvals-kpis")).toHaveTextContent("2");
    // The per-row Spend column exists and degrades to "—" for unattributed rows.
    expect(screen.getByText("Spend")).toBeInTheDocument();
    expect(screen.getAllByTestId("approval-cost")).toHaveLength(2);
  });

  it("Grant fires GrantApproval with the server-derived requestId", async () => {
    const rid = "cc".repeat(16);
    const mock = makeMockClient({
      approvalsListPending: async () => ({ approvals: [pending(rid)] }),
    });
    render(<MonitoringSection tab="approvals" />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getByTestId("approval-grant-btn")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("approval-grant-btn"));
    await waitFor(() => expect(mock.approvalsGrant).toHaveBeenCalledWith(rid, ""));
  });

  it("Deny fires DenyApproval with the requestId", async () => {
    const rid = "dd".repeat(16);
    const mock = makeMockClient({
      approvalsListPending: async () => ({ approvals: [pending(rid)] }),
    });
    render(<MonitoringSection tab="approvals" />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getByTestId("approval-deny-btn")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("approval-deny-btn"));
    await waitFor(() => expect(mock.approvalsDeny).toHaveBeenCalledWith(rid, ""));
  });

  it("shows the actionable empty state when nothing is awaiting a decision", async () => {
    const mock = makeMockClient(); // default listPending → { approvals: [] }
    render(<MonitoringSection tab="approvals" />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() =>
      expect(screen.getByText(/No actions awaiting approval/i)).toBeInTheDocument(),
    );
  });

  it("degrades to the honest not-wired note on a gateway without the approval admin", async () => {
    const mock = makeMockClient({ approvalsListPending: async () => unimplemented() });
    render(<MonitoringSection tab="approvals" />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getByTestId("approvals-not-wired")).toBeInTheDocument());
  });

  it("exposes the Approvals tab in the monitoring fieldset and reports clicks", async () => {
    const onTab = vi.fn();
    const mock = makeMockClient();
    render(<MonitoringSection tab="approvals" onTab={onTab} />, {
      wrapper: connectedWrapper(mock.client),
    });
    expect(screen.getByTestId("monitor-tab-approvals")).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByTestId("monitor-tab-alerts"));
    expect(onTab).toHaveBeenCalledWith("alerts");
  });
});

describe("ApprovalCost (RC6a per-run spend readout)", () => {
  it("renders turns / tool-calls / estimated spend from GetRunCost", async () => {
    const mock = makeMockClient({
      costGetRunCost: async () => ({
        instanceId: "aa".repeat(8),
        turns: 3,
        toolCalls: 2,
        estimatedMicroUsd: 1500,
        ceilingMicroUsd: 0,
        perTurnMicroUsd: 500,
        perToolCallMicroUsd: 0,
        overCeiling: false,
      }),
    });
    render(<ApprovalCost instanceId={"aa".repeat(8)} />, {
      wrapper: connectedWrapper(mock.client),
    });
    await waitFor(() => expect(screen.getByText(/3t\/2c/)).toBeInTheDocument());
    expect(screen.getByText(/\$0\.0015/)).toBeInTheDocument();
  });

  it("flags an over-ceiling run and shows no fabricated $ at zero-baseline pricing", async () => {
    const mock = makeMockClient({
      costGetRunCost: async () => ({
        instanceId: "bb".repeat(8),
        turns: 9,
        toolCalls: 4,
        estimatedMicroUsd: 0, // zero-baseline price book — no dollar figure to show
        ceilingMicroUsd: 0,
        perTurnMicroUsd: 0,
        perToolCallMicroUsd: 0,
        overCeiling: true,
      }),
    });
    render(<ApprovalCost instanceId={"bb".repeat(8)} />, {
      wrapper: connectedWrapper(mock.client),
    });
    await waitFor(() => expect(screen.getByText(/9t\/4c/)).toBeInTheDocument());
    expect(screen.queryByText(/\$/)).not.toBeInTheDocument(); // GR15: no invented cost
    expect(screen.getByText(/⚠/)).toBeInTheDocument();
  });

  it("degrades to a bare — for a run with no instance id", () => {
    const mock = makeMockClient();
    render(<ApprovalCost instanceId="" />, { wrapper: connectedWrapper(mock.client) });
    expect(screen.getByText("—")).toBeInTheDocument();
  });
});
