/**
 * The App project file tree — the row's job is to make the FILENAME legible.
 *
 * This surface had no test at all, which is how it shipped giving a content-hash chip
 * ~110px of a 180px rail and truncating every filename to about six characters: two
 * sibling components both rendered `Counte…`, which reads as "the project was never
 * generated". These tests pin the contract that prevents the regression class —
 * the name is present in full in the markup, the full PATH is recoverable on hover,
 * and no competing element is rendered inside the row.
 */

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { FileTree } from "../../src/components/apps/FileTree";
import { buildFileTree } from "../../src/lib/file-tree";

const REF = "c6a6e598".padEnd(64, "0");

function tree() {
  return buildFileTree([
    { path: "src/components/CounterControls.tsx", contentRef: REF },
    { path: "src/components/CounterDisplay.tsx", contentRef: REF },
    { path: "README.md", contentRef: REF },
  ]);
}

describe("FileTree", () => {
  it("renders each filename in full, so siblings are distinguishable", () => {
    render(<FileTree nodes={tree()} selectedPath={null} onSelect={vi.fn()} />);
    // The two siblings differ only after the sixth character — the exact pair the old
    // row collapsed into two identical `Counte…` labels.
    expect(screen.getByTestId("file-src/components/CounterControls.tsx")).toHaveTextContent(
      "CounterControls.tsx",
    );
    expect(screen.getByTestId("file-src/components/CounterDisplay.tsx")).toHaveTextContent(
      "CounterDisplay.tsx",
    );
  });

  it("puts the full PATH on title=, not just the leaf name", () => {
    render(<FileTree nodes={tree()} selectedPath={null} onSelect={vi.fn()} />);
    // In a nested tree the leaf alone is the least informative part of what you hover.
    expect(screen.getByTestId("file-src/components/CounterControls.tsx")).toHaveAttribute(
      "title",
      "src/components/CounterControls.tsx",
    );
    expect(screen.getByTestId("folder-src/components")).toHaveAttribute("title", "src/components");
  });

  it("renders no content-hash chip inside a row", () => {
    render(<FileTree nodes={tree()} selectedPath={null} onSelect={vi.fn()} />);
    // The ref belongs in the file-pane head, where there is width for it. A chip here is
    // `flex-shrink: 0`, so it takes its ~110px out of the filename's share — and nesting
    // its <button> inside the row <button> is invalid HTML besides.
    expect(screen.queryByTestId("digest-chip")).toBeNull();
    expect(screen.getByTestId("file-README.md").querySelector("button")).toBeNull();
  });

  it("keeps the testid on the clickable element", () => {
    const onSelect = vi.fn();
    render(<FileTree nodes={tree()} selectedPath={null} onSelect={onSelect} />);
    const row = screen.getByTestId("file-README.md");
    expect(row.tagName).toBe("BUTTON");
    row.click();
    expect(onSelect).toHaveBeenCalledWith("README.md", REF);
  });

  it("says the authoring state in words, not only as a glyph", () => {
    // A top-level file, so the state lands on the FILE node (a nested path would put it
    // on the folder that `buildFileTree` synthesises).
    const nodes = buildFileTree([{ path: "app.json", contentRef: "" }]).map((n) => ({
      ...n,
      state: "writing" as const,
    }));
    render(<FileTree nodes={nodes} selectedPath={null} onSelect={vi.fn()} />);
    expect(screen.getByTestId("file-app.json")).toHaveAttribute(
      "title",
      "app.json — being written now",
    );
  });
});
