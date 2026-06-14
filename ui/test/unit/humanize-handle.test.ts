/** PR-4.1b clean-name rule: humanize a wire handle for the card headline. */

import { describe, expect, it } from "vitest";
import { handleLeaf, humanizeHandle } from "../../src/lib/humanize-handle";

describe("humanizeHandle", () => {
  it("strips the namespace and Title-Cases the leaf", () => {
    expect(humanizeHandle("kx/recipes/echo")).toBe("Echo");
    expect(humanizeHandle("kx/recipes/agent_loop")).toBe("Agent Loop");
    expect(humanizeHandle("kx/recipes/passthrough-dag")).toBe("Passthrough Dag");
  });

  it("handles single-segment, already-clean, and trailing-slash handles", () => {
    expect(humanizeHandle("my-recipe")).toBe("My Recipe");
    expect(humanizeHandle("Echo")).toBe("Echo");
    expect(humanizeHandle("kx/recipes/")).toBe("Recipes");
  });

  it("falls back to the trimmed input on an empty leaf", () => {
    expect(humanizeHandle("")).toBe("");
    expect(humanizeHandle("   ")).toBe("");
  });
});

describe("handleLeaf", () => {
  it("returns the bare leaf for the secondary chip", () => {
    expect(handleLeaf("kx/recipes/echo")).toBe("echo");
    expect(handleLeaf("solo")).toBe("solo");
    expect(handleLeaf("kx/recipes/")).toBe("recipes");
  });
});
