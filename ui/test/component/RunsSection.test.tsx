/**
 * The Workflows section view-toggle: the runnable CATALOG (default), your one-time
 * run HISTORY (Runs), and the reusable TEMPLATES placeholder. Pins the tab routing
 * only; the heavy child bodies are stubbed (each has its own test).
 */

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("../../src/components/sections/WorkflowsCatalog", () => ({
  WorkflowsCatalog: () => <div data-testid="stub-workflows-catalog" />,
}));
vi.mock("../../src/components/sections/RunsTable", () => ({
  RunsTable: () => <div data-testid="run-list" />,
}));
vi.mock("../../src/components/sections/WorkflowsTemplatesPanel", () => ({
  WorkflowsTemplatesPanel: () => <div data-testid="workflows-templates" />,
}));
vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({ invalidateQueries: async () => {} }),
}));
vi.mock("../../src/kx/connection-context", () => ({
  useConnection: () => ({ endpoint: "test" }),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, children, ...rest }: any) => (
    <a href={typeof to === "string" ? to : "#"} {...rest}>
      {children}
    </a>
  ),
}));

import { RunsSection } from "../../src/components/sections/RunsSection";

describe("RunsSection (Workflows tabs)", () => {
  it("defaults to the catalog with the view-toggle + top-right actions", () => {
    render(<RunsSection tab="catalog" />);
    expect(screen.getByTestId("runs-section")).toBeInTheDocument();
    expect(screen.getByTestId("workflows-tabs")).toBeInTheDocument();
    expect(screen.getByTestId("workflows-refresh")).toBeInTheDocument();
    expect(screen.getByTestId("workflows-new")).toBeInTheDocument();
    expect(screen.getByTestId("stub-workflows-catalog")).toBeInTheDocument();
    expect(screen.queryByTestId("workflows-runs")).toBeNull();
    expect(screen.queryByTestId("workflows-templates")).toBeNull();
  });

  it("the Runs tab mounts the run-history table", () => {
    render(<RunsSection tab="runs" />);
    expect(screen.getByTestId("workflows-runs")).toBeInTheDocument();
    expect(screen.getByTestId("run-list")).toBeInTheDocument();
    expect(screen.queryByTestId("stub-workflows-catalog")).toBeNull();
  });

  it("the Templates tab mounts the reusable-templates placeholder", () => {
    render(<RunsSection tab="templates" />);
    expect(screen.getByTestId("workflows-templates")).toBeInTheDocument();
    expect(screen.queryByTestId("stub-workflows-catalog")).toBeNull();
  });

  it("clicking a tab reports its id", () => {
    const onTab = vi.fn();
    render(<RunsSection tab="catalog" onTab={onTab} />);
    fireEvent.click(screen.getByTestId("workflows-tab-runs"));
    expect(onTab).toHaveBeenCalledWith("runs");
  });
});
