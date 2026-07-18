import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import { describe, expect, it, vi } from "vitest";

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

import { Sidebar } from "../../src/components/shell/Sidebar";

describe("Sidebar (POC-5c / D168 flat IA)", () => {
  it("renders a plain-button item with a label for every flat section when expanded", () => {
    render(<Sidebar collapsed={false} onToggle={() => {}} />);
    for (const id of ["chat", "apps", "runs", "context", "tools", "models", "systems"]) {
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
    // The demoted sections are NOT sidebar buttons (folded into a section/tab;
    // still reachable via ⌘K + deep link).
    for (const id of ["recipes", "datasets", "branches"]) {
      expect(screen.queryByTestId(`nav-${id}`)).toBeNull();
    }
    // Activity / Artifacts were never flat sections.
    expect(screen.queryByTestId("nav-activity")).toBeNull();
    expect(screen.queryByTestId("nav-artifacts")).toBeNull();
  });

  it("hosts the New flyout trigger", () => {
    render(<Sidebar collapsed={false} onToggle={() => {}} />);
    expect(screen.getByTestId("sidebar-new")).toBeInTheDocument();
  });

  it("hides labels (icon rail) when collapsed", () => {
    render(<Sidebar collapsed={true} onToggle={() => {}} />);
    // The items remain (icons), but the text labels are not rendered.
    expect(screen.getByTestId("nav-chat")).toBeInTheDocument();
    expect(screen.queryByText("New Chat")).not.toBeInTheDocument();
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

  it("no longer badges the Apps nav item — the pending-approval count moved to the navbar bell (D213)", () => {
    render(<Sidebar collapsed={false} onToggle={() => {}} />);
    // The sidebar no longer polls approvals nor decorates any nav item (the navbar
    // ApprovalsBell owns the count now — see the approvals e2e).
    for (const id of ["apps", "chat", "runs", "context", "tools", "models", "systems"]) {
      expect(screen.queryByTestId(`nav-badge-${id}`)).toBeNull();
    }
  });
});
