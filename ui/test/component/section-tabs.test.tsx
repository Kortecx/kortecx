/**
 * POC-5c (D168) tab fold — the Context (Bundles | Datasets) section gained a
 * URL-addressable view-toggle so a demoted section keeps its capability under a
 * flat-nav home. This test pins the tab-routing logic only; the heavy child bodies
 * are stubbed (each has its own test).
 */

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

// Stub the heavy child bodies so these tests focus on tab routing. Mocks precede the
// section imports (vitest hoists them).
vi.mock("../../src/components/context/ContextBundleList", () => ({
  ContextBundleList: () => <div data-testid="stub-bundle-list" />,
}));
vi.mock("../../src/components/context/NewContextBundleForm", () => ({
  NewContextBundleForm: () => <div data-testid="stub-bundle-form" />,
}));
vi.mock("../../src/components/sections/DatasetsSection", () => ({
  DatasetsSection: () => <div data-testid="datasets-section" />,
}));
vi.mock("../../src/kx/connection-context", () => ({
  useConnection: () => ({ endpoint: "http://localhost:8888", wsEndpoint: undefined }),
}));

import { ContextSection } from "../../src/components/sections/ContextSection";

describe("ContextSection tabs (POC-5c)", () => {
  it("defaults to Bundles; the Datasets tab renders the Data Lab (two separate stores)", () => {
    const { rerender } = render(<ContextSection tab="bundles" />);
    expect(screen.getByTestId("context-section")).toBeInTheDocument();
    expect(screen.getByTestId("context-tabs")).toBeInTheDocument();
    expect(screen.getByTestId("stub-bundle-list")).toBeInTheDocument();
    expect(screen.getByTestId("context-tab-bundles")).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByTestId("datasets-section")).toBeNull();

    rerender(<ContextSection tab="datasets" />);
    expect(screen.getByTestId("datasets-section")).toBeInTheDocument();
    expect(screen.queryByTestId("stub-bundle-list")).toBeNull();
    expect(screen.getByTestId("context-tab-datasets")).toHaveAttribute("aria-pressed", "true");
  });

  it("clicking a tab calls onTab with its id", () => {
    const onTab = vi.fn();
    render(<ContextSection tab="bundles" onTab={onTab} />);
    fireEvent.click(screen.getByTestId("context-tab-datasets"));
    expect(onTab).toHaveBeenCalledWith("datasets");
  });
});
