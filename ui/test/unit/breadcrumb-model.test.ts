import { describe, expect, it } from "vitest";
import { deriveCrumbs } from "../../src/components/shell/breadcrumb-model";

describe("deriveCrumbs", () => {
  it("maps each nav section path to a single current crumb with its display label", () => {
    // The spec-IA display renames (§2.186 / D141) over the frozen handles.
    expect(deriveCrumbs("/chat")).toEqual([{ label: "New Chat" }]);
    expect(deriveCrumbs("/workflows")).toEqual([{ label: "Workflows" }]);
    // The display rename (D136): the frozen `recipes` path shows "Blueprints".
    expect(deriveCrumbs("/recipes")).toEqual([{ label: "Blueprints" }]);
    expect(deriveCrumbs("/systems")).toEqual([{ label: "Security" }]);
    expect(deriveCrumbs("/context")).toEqual([{ label: "Context" }]);
    expect(deriveCrumbs("/settings")).toEqual([{ label: "Settings" }]);
  });

  it("retired redirect-only routes no longer breadcrumb (PR-2 merge)", () => {
    expect(deriveCrumbs("/activity")).toEqual([]);
    expect(deriveCrumbs("/runs")).toEqual([]);
    expect(deriveCrumbs("/artifacts")).toEqual([]);
  });

  it("renders a linked section crumb plus a short-hex detail crumb on run detail", () => {
    // A run instance id is 16 bytes = 32 hex chars (PR-2 widened HEX_ID).
    const instance = "ab12cd34".repeat(4);
    expect(deriveCrumbs(`/workflows/${instance}`)).toEqual([
      { label: "Workflows", path: "/workflows" },
      { label: "ab12cd34…cd34" },
    ]);
    // 32-byte (64 hex) ids shorten too.
    const mote = "ab12cd34".repeat(8);
    expect(deriveCrumbs(`/workflows/${mote}`)).toEqual([
      { label: "Workflows", path: "/workflows" },
      { label: "ab12cd34…cd34" },
    ]);
  });

  it("keeps a non-id segment verbatim", () => {
    expect(deriveCrumbs("/workflows/latest")).toEqual([
      { label: "Workflows", path: "/workflows" },
      { label: "latest" },
    ]);
  });

  it("returns no crumbs outside the nav model (gate/root paths)", () => {
    expect(deriveCrumbs("/")).toEqual([]);
    expect(deriveCrumbs("/connect")).toEqual([]);
  });

  it("does not prefix-match across section boundaries", () => {
    // `/runsxyz` must not match `/runs`.
    expect(deriveCrumbs("/runsxyz")).toEqual([]);
  });
});
