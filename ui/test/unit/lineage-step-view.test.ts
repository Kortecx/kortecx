/**
 * The App Lineage per-step view-model — pure, so every branch is asserted here rather
 * than through the DOM.
 *
 * The `entryAgenticStepId` block is a DRIFT GUARD: it mirrors, case for case, the Rust
 * tests for `entry_agentic_step_index` / `fold_skill_tools` in
 * `crates/kx-gateway/src/app_run.rs` — the server function this UI claims to reproduce.
 * If the server's rule ever changes, these fail rather than the console quietly pointing
 * the ⚑ entry badge at the wrong step.
 */

import { describe, expect, it } from "vitest";
import type {
  BuilderEdge,
  BuilderGraph,
  BuilderStep,
} from "../../src/components/builder/builder-graph";
import { newStep } from "../../src/components/builder/builder-graph";
import type { LineageStepView } from "../../src/components/sections/lineage-step-view";
import {
  diagramLabel,
  entryAgenticStepId,
  entryFoldWarning,
  lineageStepViews,
} from "../../src/components/sections/lineage-step-view";

function model(id: string, over: Partial<BuilderStep> = {}): BuilderStep {
  return { ...newStep("model", id), prompt: "go", ...over };
}
function pure(id: string, over: Partial<BuilderStep> = {}): BuilderStep {
  return { ...newStep("pure", id), ...over };
}
function tool(id: string, over: Partial<BuilderStep> = {}): BuilderStep {
  return { ...newStep("tool", id), toolId: "mcp-echo/echo", toolVersion: "1", ...over };
}
function edge(source: string, target: string): BuilderEdge {
  return { id: `e-${source}-${target}`, source, target, edge: "data", instruction: "" };
}
function graph(steps: BuilderStep[], edges: BuilderEdge[] = []): BuilderGraph {
  return { steps, edges };
}
/** The `i`-th view, asserted present (indexed access is checked in this tsconfig). */
function at(views: readonly LineageStepView[], i: number): LineageStepView {
  const v = views[i];
  if (v === undefined) {
    throw new Error(`no view at index ${i}`);
  }
  return v;
}

describe("entryAgenticStepId (mirrors the server's entry_agentic_step_index)", () => {
  it("targets the FIRST root model step when several are roots", () => {
    // Mirrors `fold_skill_tools_targets_the_entry_root_model_step_and_declared_version_wins`:
    // two model steps, no edges ⇒ both are roots ⇒ only the first is the entry.
    expect(entryAgenticStepId(graph([model("s0"), model("s1")]))).toBe("s0");
  });

  it("returns null for a pure → model chain (the split the server refuses)", () => {
    // Mirrors `fold_skill_tools_refuses_the_split_when_the_model_step_is_not_a_root`:
    // the instructions bind to the PURE root, so folding tools onto the non-root model
    // step would grant tools whose instructions are unreachable. No entry ⇒ no fold.
    const g = graph([pure("s0"), model("s1")], [edge("s0", "s1")]);
    expect(entryAgenticStepId(g)).toBeNull();
  });

  it("treats a single model step as a root", () => {
    // The second half of the same Rust test: `entry_agentic_step_index(&single) == Some(0)`.
    expect(entryAgenticStepId(graph([model("s0")]))).toBe("s0");
  });

  it("returns null when the blueprint has no model step at all", () => {
    expect(entryAgenticStepId(graph([pure("s0"), tool("s1")]))).toBeNull();
  });

  it("skips a non-root model step in favour of a later root model step", () => {
    // s1 is a model step but has an inbound edge; s2 is a model root ⇒ s2 is the entry.
    const g = graph([pure("s0"), model("s1"), model("s2")], [edge("s0", "s1")]);
    expect(entryAgenticStepId(g)).toBe("s2");
  });

  it("ignores a tool step that is a root", () => {
    expect(entryAgenticStepId(graph([tool("s0"), model("s1")]))).toBe("s1");
  });
});

describe("entryFoldWarning", () => {
  it("warns when wishes exist but no root model step can carry them", () => {
    const g = graph([pure("s0"), model("s1")], [edge("s0", "s1")]);
    expect(entryFoldWarning(g, 3)).toContain("no root agent step");
  });

  it("singularizes one wish, pronouns included", () => {
    const msg = entryFoldWarning(graph([pure("s0")]), 1) ?? "";
    expect(msg).toContain("1 skill/tool wish can't");
    expect(msg).toContain("fold it onto");
    expect(msg).toContain("drops it.");
  });

  it("pluralizes several wishes", () => {
    const msg = entryFoldWarning(graph([pure("s0")]), 3) ?? "";
    expect(msg).toContain("3 skill/tool wishes can't");
    expect(msg).toContain("drops them.");
  });

  it("stays silent when there is an entry step to fold onto", () => {
    expect(entryFoldWarning(graph([model("s0")]), 3)).toBeNull();
  });

  it("stays silent when the App declares no wishes at all", () => {
    expect(entryFoldWarning(graph([pure("s0")]), 0)).toBeNull();
  });
});

describe("lineageStepViews — derived title", () => {
  it("prefers the prompt's first non-empty line", () => {
    const v = lineageStepViews(
      graph([model("s0", { prompt: "\n  Research the target\nmore" })]),
      "",
    );
    expect(at(v, 0).title).toBe("Research the target");
  });

  it("ellipsizes a long first line", () => {
    const long = "x".repeat(80);
    const v = lineageStepViews(graph([model("s0", { prompt: long })]), "");
    expect(at(v, 0).title.endsWith("…")).toBe(true);
    expect(at(v, 0).title.length).toBeLessThanOrEqual(48);
  });

  it("falls back to the model id when there is no prompt", () => {
    const v = lineageStepViews(graph([model("s0", { prompt: "", modelId: "gemma-4-12b" })]), "");
    expect(at(v, 0).title).toBe("gemma-4-12b");
  });

  it("falls back to tool@version for a tool step", () => {
    const v = lineageStepViews(graph([tool("s0")]), "");
    expect(at(v, 0).title).toBe("mcp-echo/echo@1");
  });

  it("falls back to the ordinal — never blank", () => {
    const v = lineageStepViews(graph([pure("s0"), pure("s1")]), "");
    expect(at(v, 1).title).toBe("Step 2");
  });

  it("keeps the full prompt as the tooltip", () => {
    const v = lineageStepViews(graph([model("s0", { prompt: "line one\nline two" })]), "");
    expect(at(v, 0).tooltip).toBe("line one\nline two");
  });
});

describe("lineageStepViews — model line", () => {
  it("shows an explicitly bound model", () => {
    const v = lineageStepViews(graph([model("s0", { modelId: "gemma-4-12b" })]), "kx-serve:x");
    expect(at(v, 0).model).toBe("gemma-4-12b");
    expect(at(v, 0).modelInferred).toBe(false);
  });

  it("defers to the App's model_route when the step names none", () => {
    const v = lineageStepViews(graph([model("s0")]), "kx-serve:gemma");
    expect(at(v, 0).model).toBe("inherits kx-serve:gemma");
    expect(at(v, 0).modelInferred).toBe(true);
  });

  it("defers to the served model when there is no route either", () => {
    const v = lineageStepViews(graph([model("s0")]), "");
    expect(at(v, 0).model).toBe("served model at run");
    expect(at(v, 0).modelInferred).toBe(true);
  });

  it("shows no model line for a pure step", () => {
    expect(at(lineageStepViews(graph([pure("s0")]), "kx-serve:x"), 0).model).toBeNull();
  });
});

describe("lineageStepViews — requested tools", () => {
  it("chips a model step's ReAct contract and counts the overflow", () => {
    const v = lineageStepViews(
      graph([model("s0", { toolContract: { a: "1", b: "1", c: "1", d: "1" } })]),
      "",
    );
    expect(at(v, 0).tools.map((t) => t.id)).toEqual(["a", "b"]);
    expect(at(v, 0).toolsOverflow).toBe(2);
  });

  it("chips a tool step's single registered tool", () => {
    const v = lineageStepViews(graph([tool("s0")]), "");
    expect(at(v, 0).tools).toEqual([{ id: "mcp-echo/echo", version: "1" }]);
    expect(at(v, 0).toolsOverflow).toBe(0);
  });

  it("requests nothing for a pure step", () => {
    expect(at(lineageStepViews(graph([pure("s0")]), ""), 0).tools).toEqual([]);
  });
});

describe("lineageStepViews — budget (never synthesized)", () => {
  it("renders only what the blueprint explicitly carries", () => {
    const v = lineageStepViews(graph([model("s0", { maxTurns: 8, maxToolCalls: 6 })]), "");
    expect(at(v, 0).budget).toBe("8 turns · 6 calls");
  });

  it("renders a partial budget when only one bound is authored", () => {
    expect(at(lineageStepViews(graph([model("s0", { maxTurns: 4 })]), ""), 0).budget).toBe(
      "4 turns",
    );
  });

  it("renders NO budget when the blueprint omits both (the default is contested in-tree)", () => {
    expect(at(lineageStepViews(graph([model("s0")]), ""), 0).budget).toBeNull();
  });
});

describe("diagramLabel (role=img hides the cards from AT — the label must carry them)", () => {
  it("names every step, its model, its requested tools and the entry step", () => {
    const g = graph(
      [
        model("s0", {
          prompt: "Plan the work",
          modelId: "gemma-4-12b",
          toolContract: { "web/search": "1" },
        }),
        model("s1", { prompt: "Write it up" }),
      ],
      [edge("s0", "s1")],
    );
    const label = diagramLabel(lineageStepViews(g, "kx-serve:gemma"));
    expect(label).toContain("2 steps");
    expect(label).toContain("Step 1, model: Plan the work");
    expect(label).toContain("model gemma-4-12b");
    expect(label).toContain("requests web/search");
    expect(label).toContain("the entry step");
    expect(label).toContain("Step 2, model: Write it up");
  });

  it("says so when there is nothing to describe", () => {
    expect(diagramLabel([])).toBe("App structure diagram: no steps.");
  });
});

describe("lineageStepViews — ordinal + entry flag", () => {
  it("numbers steps from 1 and flags only the entry step", () => {
    const v = lineageStepViews(graph([model("s0"), model("s1")]), "");
    expect(v.map((s) => s.ordinal)).toEqual([1, 2]);
    expect(v.map((s) => s.isEntry)).toEqual([true, false]);
  });

  it("flags nothing when the blueprint has no root model step", () => {
    const v = lineageStepViews(graph([pure("s0"), model("s1")], [edge("s0", "s1")]), "");
    expect(v.some((s) => s.isEntry)).toBe(false);
  });
});
