/**
 * PR-D: the high-level step-type classifier for the read-only run review.
 */

import { describe, expect, it } from "vitest";
import { STEP_LABEL, classifyStep } from "../../src/lib/step-kind";

describe("classifyStep (PR-D)", () => {
  it("a MODEL step → model (raw enum or friendly kind)", () => {
    expect(classifyStep("MODEL", {})).toBe("model");
    expect(classifyStep("WORKFLOW_STEP_KIND_MODEL", {})).toBe("model");
    // model wins even if the step also carries a tool contract.
    expect(classifyStep("MODEL", { "web-search": "1" })).toBe("model");
  });

  it("a TOOL step with an MCP tool (server/tool or mcp-*) → mcp", () => {
    expect(classifyStep("TOOL", { "mcp-echo/echo": "1" })).toBe("mcp");
    expect(classifyStep("TOOL", { "docs/search": "1" })).toBe("mcp");
  });

  it("a TOOL step naming a community connector → connector", () => {
    expect(classifyStep("TOOL", { gmail: "1" })).toBe("connector");
    expect(classifyStep("TOOL", { "slack-send": "1" })).toBe("connector");
  });

  it("a TOOL step with a generic registered tool → tool", () => {
    expect(classifyStep("TOOL", { "web-search": "1" })).toBe("tool");
    // A tool contract with no explicit TOOL kind still classifies by the tool.
    expect(classifyStep("", { "web-search": "1" })).toBe("tool");
  });

  it("PURE / EXEC → action; empty / unrecognized → unknown", () => {
    expect(classifyStep("PURE", {})).toBe("action");
    expect(classifyStep("EXEC", {})).toBe("action");
    expect(classifyStep("", {})).toBe("unknown");
    expect(classifyStep("SOMETHING_ELSE", {})).toBe("unknown");
  });

  it("every step type has a display label", () => {
    for (const t of ["model", "mcp", "connector", "tool", "action", "unknown"] as const) {
      expect(STEP_LABEL[t]).toBeTruthy();
    }
  });
});
