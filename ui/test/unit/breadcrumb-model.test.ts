import { describe, expect, it } from "vitest";
import { deriveCrumbs } from "../../src/components/shell/breadcrumb-model";

describe("deriveCrumbs", () => {
  it("maps each nav section path to a single current crumb with its display label", () => {
    expect(deriveCrumbs("/activity")).toEqual([{ label: "Activity" }]);
    expect(deriveCrumbs("/chat")).toEqual([{ label: "Chat" }]);
    // The display rename (D136): the frozen `recipes` path shows "Blueprints".
    expect(deriveCrumbs("/recipes")).toEqual([{ label: "Blueprints" }]);
    expect(deriveCrumbs("/settings")).toEqual([{ label: "Settings" }]);
  });

  it("renders a linked section crumb plus a short-hex detail crumb on run detail", () => {
    const id = "ab12cd34".repeat(8); // 64 hex chars
    expect(deriveCrumbs(`/runs/${id}`)).toEqual([
      { label: "Runs", path: "/runs" },
      { label: "ab12cd34…cd34" },
    ]);
  });

  it("keeps a non-id segment verbatim", () => {
    expect(deriveCrumbs("/runs/latest")).toEqual([
      { label: "Runs", path: "/runs" },
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
