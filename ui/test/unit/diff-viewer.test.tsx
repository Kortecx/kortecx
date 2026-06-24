/**
 * POC-5d: the agentic-edit review gate's diff surface. The pure `lineDiff`/`isNoOpDiff`
 * helpers + the headless (jsdom) fallback render — assertable without real Monaco.
 */

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { DiffViewer, isNoOpDiff, lineDiff } from "../../src/components/editor/DiffViewer";

describe("lineDiff (pure LCS)", () => {
  it("marks added and removed lines, keeps unchanged", () => {
    const d = lineDiff("a\nb\nc", "a\nB\nc");
    expect(d).toEqual([
      { kind: "same", text: "a" },
      { kind: "del", text: "b" },
      { kind: "add", text: "B" },
      { kind: "same", text: "c" },
    ]);
  });

  it("an identical document is all 'same'", () => {
    const d = lineDiff("x\ny", "x\ny");
    expect(d.every((l) => l.kind === "same")).toBe(true);
  });

  it("pure insertion / deletion at the tail", () => {
    expect(lineDiff("a", "a\nb")).toEqual([
      { kind: "same", text: "a" },
      { kind: "add", text: "b" },
    ]);
    expect(lineDiff("a\nb", "a")).toEqual([
      { kind: "same", text: "a" },
      { kind: "del", text: "b" },
    ]);
  });

  it("isNoOpDiff detects an identical proposal", () => {
    expect(isNoOpDiff("x", "x")).toBe(true);
    expect(isNoOpDiff("x", "y")).toBe(false);
  });
});

describe("DiffViewer fallback (headless)", () => {
  it("renders the line-diff fallback with added/removed lines", () => {
    render(<DiffViewer original={"a\nb"} modified={"a\nc"} language="plaintext" />);
    const pre = screen.getByTestId("app-diff-fallback");
    expect(pre).toBeInTheDocument();
    const lines = pre.querySelectorAll("[data-diff-kind]");
    const kinds = Array.from(lines).map((n) => n.getAttribute("data-diff-kind"));
    expect(kinds).toContain("add");
    expect(kinds).toContain("del");
  });

  it("a no-op proposal shows the no-op note (Approve is meaningless)", () => {
    render(<DiffViewer original="same" modified="same" language="plaintext" />);
    expect(screen.getByTestId("app-edit-noop")).toBeInTheDocument();
    expect(screen.queryByTestId("app-diff-fallback")).toBeNull();
  });
});
