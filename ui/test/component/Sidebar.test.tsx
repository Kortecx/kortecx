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

// The token footer reads live telemetry (react-query + connection); stub the hook
// so the sidebar renders without a provider tree (the footer's own test covers it).
vi.mock("../../src/kx/use-telemetry", () => ({
  useTelemetry: () => ({ rows: [], notWired: false, isLoading: false }),
}));

import { Sidebar } from "../../src/components/shell/Sidebar";

describe("Sidebar", () => {
  it("renders an item with a label for every section when expanded", () => {
    render(<Sidebar collapsed={false} onToggle={() => {}} />);
    for (const id of ["chat", "runs", "recipes", "datasets", "tools", "context", "systems"]) {
      expect(screen.getByTestId(`nav-${id}`)).toBeInTheDocument();
    }
    expect(screen.getByText("New Chat")).toBeInTheDocument();
    expect(screen.getByText("Context")).toBeInTheDocument();
    // The display renames over frozen ids (D136 / D141): recipes shows
    // "Blueprints", runs shows "Workflows", systems shows "Security".
    expect(screen.getByTestId("nav-recipes")).toHaveTextContent("Blueprints");
    expect(screen.getByTestId("nav-runs")).toHaveTextContent("Workflows");
    expect(screen.getByTestId("nav-systems")).toHaveTextContent("Security");
    // Activity left the sidebar (it is the navbar drawer now).
    expect(screen.queryByTestId("nav-activity")).not.toBeInTheDocument();
    expect(screen.queryByTestId("nav-artifacts")).not.toBeInTheDocument();
    expect(screen.getByTestId("sidebar")).toHaveAttribute("data-collapsed", "false");
    // The sidebar hosts the console's single brand anchor (icon + wordmark).
    expect(screen.getByTestId("brand")).toHaveTextContent("kortecx");
  });

  it("groups the sections with coloured labels (PR-B / D150)", () => {
    render(<Sidebar collapsed={false} onToggle={() => {}} />);
    for (const g of ["workspace", "data", "tools", "monitoring", "security", "cloud"]) {
      expect(screen.getByTestId(`nav-group-${g}`)).toBeInTheDocument();
    }
    // Group labels render when expanded (scoped to the group container — single-
    // section groups share a name with their item, so the testid disambiguates).
    expect(screen.getByTestId("nav-group-workspace")).toHaveTextContent("Workspace");
    expect(screen.getByTestId("nav-group-data")).toHaveTextContent("Data");
    expect(screen.getByTestId("nav-group-cloud")).toHaveTextContent("Cloud");
  });

  it("hosts the New flyout trigger and the token footer", () => {
    render(<Sidebar collapsed={false} onToggle={() => {}} />);
    expect(screen.getByTestId("sidebar-new")).toBeInTheDocument();
    expect(screen.getByTestId("token-usage")).toBeInTheDocument();
    // No telemetry rows in the stub → honest-empty readout (never a fabricated 0).
    expect(screen.getByTestId("token-usage-empty")).toBeInTheDocument();
  });

  it("renders Cloud placeholders as honest-disabled (never navigable)", () => {
    render(<Sidebar collapsed={false} onToggle={() => {}} />);
    for (const id of ["sharing", "federation", "experts"]) {
      const el = screen.getByTestId(`cloud-${id}`);
      expect(el).toBeInTheDocument();
      expect(el).toHaveAttribute("aria-disabled", "true");
      // Not a link / button — a plain greyed entry.
      expect(el.tagName).toBe("DIV");
    }
  });

  it("has no in-development placeholders now (Apps POC-4, Policies POC-5b promoted)", () => {
    render(<Sidebar collapsed={false} onToggle={() => {}} />);
    // DEV_PLACEHOLDERS is empty — the "Coming" group is hidden, no dev rows.
    expect(screen.getByTestId("nav-group-dev")).toHaveAttribute("hidden");
    expect(screen.queryByTestId("dev-policies")).toBeNull();
    expect(screen.queryByTestId("dev-apps")).toBeNull();
    // Both are real, navigable sections now.
    expect(screen.getByTestId("nav-apps")).toBeInTheDocument();
    expect(screen.getByTestId("nav-policies")).toBeInTheDocument();
  });

  it("hides labels, group labels and the footer (icon rail) when collapsed", () => {
    render(<Sidebar collapsed={true} onToggle={() => {}} />);
    // The items remain (icons), but the text labels are not rendered.
    expect(screen.getByTestId("nav-chat")).toBeInTheDocument();
    expect(screen.queryByText("New Chat")).not.toBeInTheDocument();
    // Group labels and the token footer drop away on the rail.
    expect(screen.getByTestId("nav-group-workspace")).not.toHaveTextContent("Workspace");
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
});
