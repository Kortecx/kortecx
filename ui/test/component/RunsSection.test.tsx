/**
 * The Workflows section view-toggle: the runnable CATALOG (default), your own run
 * HISTORY (Runs), and the self-correction TRAILS. Pins the tab routing only; the heavy
 * child bodies are stubbed (each has its own test).
 */

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("../../src/components/sections/WorkflowsTable", () => ({
  WorkflowsTable: () => <div data-testid="stub-workflows-table" />,
}));
vi.mock("../../src/components/sections/RunsTable", () => ({
  RunsTable: () => <div data-testid="run-list" />,
}));
vi.mock("../../src/components/sections/WorkflowTrails", () => ({
  WorkflowTrails: () => <div data-testid="workflows-trails" />,
}));
vi.mock("../../src/components/apps/AppRunDrawer", () => ({
  AppRunDrawer: () => <div data-testid="stub-run-drawer" />,
}));
vi.mock("../../src/kx/use-apps", () => ({ useApps: () => ({ apps: [], notWired: false }) }));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, children, ...rest }: any) => (
    <a href={typeof to === "string" ? to : "#"} {...rest}>
      {children}
    </a>
  ),
}));

import { RunsSection } from "../../src/components/sections/RunsSection";

describe("RunsSection (Workflows tabs)", () => {
  it("defaults to the catalog with the view-toggle", () => {
    render(<RunsSection tab="catalog" />);
    expect(screen.getByTestId("runs-section")).toBeInTheDocument();
    expect(screen.getByTestId("workflows-tabs")).toBeInTheDocument();
    expect(screen.getByTestId("stub-workflows-table")).toBeInTheDocument();
    expect(screen.queryByTestId("workflows-runs")).toBeNull();
    expect(screen.queryByTestId("workflows-trails")).toBeNull();
  });

  it("the Runs tab mounts the run-history table", () => {
    render(<RunsSection tab="runs" />);
    expect(screen.getByTestId("workflows-runs")).toBeInTheDocument();
    expect(screen.getByTestId("run-list")).toBeInTheDocument();
    expect(screen.queryByTestId("stub-workflows-table")).toBeNull();
  });

  it("the Trails tab mounts the self-correction trails", () => {
    render(<RunsSection tab="trails" />);
    expect(screen.getByTestId("workflows-trails")).toBeInTheDocument();
  });

  it("clicking a tab reports its id", () => {
    const onTab = vi.fn();
    render(<RunsSection tab="catalog" onTab={onTab} />);
    fireEvent.click(screen.getByTestId("workflows-tab-runs"));
    expect(onTab).toHaveBeenCalledWith("runs");
  });
});
