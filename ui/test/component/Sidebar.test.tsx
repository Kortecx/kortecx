import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The sidebar renders TanStack <Link>s; stub them to plain <a> so we can test the
// nav in isolation (the real router integration is covered by the e2e shell-nav).
vi.mock("@tanstack/react-router", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@tanstack/react-router")>();
  return {
    ...actual,
    Link: ({ to, children, activeProps, ...rest }: any) =>
      React.createElement("a", { href: typeof to === "string" ? to : "#", ...rest }, children),
  };
});

// The token footer reads live telemetry (react-query + connection); stub the hook
// so the sidebar renders without a provider tree (the footer's own test covers it).
vi.mock("../../src/kx/use-telemetry", () => ({
  useTelemetry: () => ({ rows: [], notWired: false, isLoading: false }),
}));

// RC6a: the Monitoring nav badge reads pending approvals (react-query + connection).
// Stub it (overridable per test) so the sidebar renders without a provider tree.
const { mockPending } = vi.hoisted(() => ({
  mockPending: vi.fn(() => ({ count: 0, approvals: [], notWired: false })),
}));
vi.mock("../../src/kx/use-approvals", () => ({ useListPendingApprovals: mockPending }));

import { Sidebar } from "../../src/components/shell/Sidebar";

describe("Sidebar (POC-5c / D168 flat IA)", () => {
  beforeEach(() => {
    mockPending.mockReturnValue({ count: 0, approvals: [], notWired: false });
  });

  it("renders a plain-button item with a label for every flat section when expanded", () => {
    render(<Sidebar collapsed={false} onToggle={() => {}} />);
    for (const id of ["chat", "apps", "runs", "context", "tools", "models", "monitor", "systems"]) {
      expect(screen.getByTestId(`nav-${id}`)).toBeInTheDocument();
    }
    expect(screen.getByText("New Chat")).toBeInTheDocument();
    expect(screen.getByText("Context")).toBeInTheDocument();
    // The display renames over frozen ids (D136 / D141): runs shows "Workflows",
    // systems shows "Security".
    expect(screen.getByTestId("nav-runs")).toHaveTextContent("Workflows");
    expect(screen.getByTestId("nav-systems")).toHaveTextContent("Security");
    expect(screen.getByTestId("sidebar")).toHaveAttribute("data-collapsed", "false");
    // The sidebar hosts the console's single brand anchor (icon + wordmark).
    expect(screen.getByTestId("brand")).toHaveTextContent("kortecx");
  });

  it("is a FLAT list — no groups, no Cloud/Coming placeholders, no demoted buttons", () => {
    render(<Sidebar collapsed={false} onToggle={() => {}} />);
    // Groups + placeholder constructs are gone (POC-5c).
    for (const g of ["workspace", "data", "tools", "monitoring", "security", "cloud", "dev"]) {
      expect(screen.queryByTestId(`nav-group-${g}`)).toBeNull();
    }
    for (const p of ["sharing", "federation", "experts"]) {
      expect(screen.queryByTestId(`cloud-${p}`)).toBeNull();
    }
    // The five demoted sections are NOT sidebar buttons (folded into a section/tab;
    // still reachable via ⌘K + deep link).
    for (const id of ["dashboard", "recipes", "datasets", "branches", "policies"]) {
      expect(screen.queryByTestId(`nav-${id}`)).toBeNull();
    }
    // Activity / Artifacts were never flat sections.
    expect(screen.queryByTestId("nav-activity")).toBeNull();
    expect(screen.queryByTestId("nav-artifacts")).toBeNull();
  });

  it("hosts the New flyout trigger and the token footer", () => {
    render(<Sidebar collapsed={false} onToggle={() => {}} />);
    expect(screen.getByTestId("sidebar-new")).toBeInTheDocument();
    expect(screen.getByTestId("token-usage")).toBeInTheDocument();
    // No telemetry rows in the stub → honest-empty readout (never a fabricated 0).
    expect(screen.getByTestId("token-usage-empty")).toBeInTheDocument();
  });

  it("hides labels and the footer (icon rail) when collapsed", () => {
    render(<Sidebar collapsed={true} onToggle={() => {}} />);
    // The items remain (icons), but the text labels are not rendered.
    expect(screen.getByTestId("nav-chat")).toBeInTheDocument();
    expect(screen.queryByText("New Chat")).not.toBeInTheDocument();
    // The token footer drops away on the rail.
    expect(screen.queryByTestId("token-usage")).not.toBeInTheDocument();
    expect(screen.getByTestId("sidebar")).toHaveAttribute("data-collapsed", "true");
    // Collapsed rail keeps the brand icon, drops the wordmark.
    expect(screen.getByTestId("brand")).toBeInTheDocument();
    expect(screen.queryByText("kortecx")).not.toBeInTheDocument();
  });

  it("pins Settings at the bottom and hosts the collapse toggle", () => {
    const onToggle = vi.fn();
    render(<Sidebar collapsed={false} onToggle={onToggle} />);
    expect(screen.getByTestId("nav-settings")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("sidebar-toggle"));
    expect(onToggle).toHaveBeenCalledTimes(1);
  });

  it("badges the Monitoring nav item with the pending-approval count (RC6a)", () => {
    mockPending.mockReturnValue({ count: 3, approvals: [], notWired: false });
    render(<Sidebar collapsed={false} onToggle={() => {}} />);
    expect(screen.getByTestId("nav-badge-monitor")).toHaveTextContent("3");
    // Only Monitoring carries the badge — no other section is decorated.
    for (const id of ["chat", "apps", "runs", "context", "tools", "models", "systems"]) {
      expect(screen.queryByTestId(`nav-badge-${id}`)).toBeNull();
    }
  });

  it("shows no nav badge when nothing is awaiting approval", () => {
    mockPending.mockReturnValue({ count: 0, approvals: [], notWired: false });
    render(<Sidebar collapsed={false} onToggle={() => {}} />);
    expect(screen.queryByTestId("nav-badge-monitor")).toBeNull();
  });

  it("keeps the badge on the collapsed icon rail (rides the icon)", () => {
    mockPending.mockReturnValue({ count: 7, approvals: [], notWired: false });
    render(<Sidebar collapsed={true} onToggle={() => {}} />);
    expect(screen.getByTestId("nav-badge-monitor")).toHaveTextContent("7");
  });
});
