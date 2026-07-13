/** PR-4.1b BlueprintCard — headline/subtitle/tags, the per-card action menu
 *  (Cloud-disabled Share/Schedule, Edit-in-builder), and export wiring. */

import { RecipeInfo } from "@kortecx/sdk/web";
import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, search, params, activeProps, children, ...rest }: any) =>
    React.createElement("a", { href: typeof to === "string" ? to : "#", ...rest }, children),
}));
vi.mock("../../src/kx/connection-context", () => ({
  useConnection: () => ({ endpoint: "http://ep" }),
}));

const exportBlueprint = vi.fn(() => Promise.resolve());
vi.mock("../../src/kx/use-blueprint-export", () => ({
  useBlueprintExport: () => ({ exportBlueprint, pendingHandle: null, error: null }),
}));

import { BlueprintCard } from "../../src/components/sections/BlueprintCard";

const summary = new RecipeInfo(
  "kx/recipes/echo",
  "ef".repeat(16),
  "Echoes the topic",
  ["util"],
  "1",
);

afterEach(() => {
  exportBlueprint.mockClear();
  localStorage.clear();
});

function renderCard(props: Partial<Parameters<typeof BlueprintCard>[0]> = {}) {
  const onRun = vi.fn();
  const onView = vi.fn();
  render(
    <BlueprintCard
      handle="kx/recipes/echo"
      headline="Echo"
      customName={null}
      summary={summary}
      onRun={onRun}
      onView={onView}
      {...props}
    />,
  );
  return { onRun, onView };
}

describe("BlueprintCard", () => {
  it("renders the headline, description subtitle, tags and raw handle chip", () => {
    renderCard();
    expect(screen.getByTestId("recipe-pick-kx/recipes/echo")).toHaveTextContent("Echo");
    expect(screen.getByText("Echoes the topic")).toBeInTheDocument();
    expect(screen.getByText("util")).toBeInTheDocument();
    expect(screen.getAllByText("kx/recipes/echo").length).toBeGreaterThan(0);
  });

  it("opens the form via the title and the Run menu item", () => {
    const { onRun } = renderCard();
    fireEvent.click(screen.getByTestId("recipe-pick-kx/recipes/echo"));
    expect(onRun).toHaveBeenCalledWith("kx/recipes/echo");
    fireEvent.click(screen.getByTestId("blueprint-menu"));
    fireEvent.click(screen.getByTestId("recipe-run-kx/recipes/echo"));
    expect(onRun).toHaveBeenCalledTimes(2);
  });

  it("View contract calls onView; Edit-in-builder links to the builder", () => {
    const { onView } = renderCard();
    fireEvent.click(screen.getByTestId("blueprint-menu"));
    // Edit-in-builder targets the visual builder (assert before View closes the menu).
    expect(screen.getByTestId("blueprint-edit")).toHaveAttribute("href", "/blueprints/new");
    fireEvent.click(screen.getByTestId("recipe-view-kx/recipes/echo"));
    expect(onView).toHaveBeenCalledWith("kx/recipes/echo");
  });

  it("Share stays an honest-disabled Cloud chip; the stale Schedule Cloud chip is gone", () => {
    renderCard();
    fireEvent.click(screen.getByTestId("blueprint-menu"));
    const share = screen.getByTestId("blueprint-share");
    expect(share).toBeDisabled();
    expect(share).toHaveTextContent("Cloud");
    // Scheduling is LOCAL (CRON triggers ship) — no more misleading "Schedule · Cloud".
    expect(screen.queryByTestId("blueprint-schedule")).toBeNull();
  });

  it("Export sends the blueprint's metadata to the exporter", () => {
    renderCard();
    fireEvent.click(screen.getByTestId("blueprint-menu"));
    fireEvent.click(screen.getByTestId("blueprint-export"));
    expect(exportBlueprint).toHaveBeenCalledWith({
      handle: "kx/recipes/echo",
      description: "Echoes the topic",
      tags: ["util"],
      version: "1",
    });
  });

  it("a local rename wins over the humanized handle", () => {
    renderCard({ headline: "My Blueprint", customName: "My Blueprint" });
    expect(screen.getByTestId("recipe-pick-kx/recipes/echo")).toHaveTextContent("My Blueprint");
  });
});
