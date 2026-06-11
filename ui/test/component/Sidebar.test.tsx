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

describe("Sidebar", () => {
  it("renders an item with a label for every section when expanded", () => {
    render(<Sidebar collapsed={false} onToggle={() => {}} />);
    for (const id of ["activity", "chat", "runs", "recipes", "artifacts", "datasets", "systems"]) {
      expect(screen.getByTestId(`nav-${id}`)).toBeInTheDocument();
    }
    expect(screen.getByText("Activity")).toBeInTheDocument();
    expect(screen.getByText("Chat")).toBeInTheDocument();
    // The display rename (D136): the frozen `recipes` id shows "Blueprints".
    expect(screen.getByTestId("nav-recipes")).toHaveTextContent("Blueprints");
    expect(screen.getByTestId("sidebar")).toHaveAttribute("data-collapsed", "false");
    // The sidebar hosts the console's single brand anchor (icon + wordmark).
    expect(screen.getByTestId("brand")).toHaveTextContent("kortecx");
  });

  it("hides labels (icon rail) when collapsed", () => {
    render(<Sidebar collapsed={true} onToggle={() => {}} />);
    // The items remain (icons), but the text labels are not rendered.
    expect(screen.getByTestId("nav-activity")).toBeInTheDocument();
    expect(screen.queryByText("Activity")).not.toBeInTheDocument();
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
