import { describe, expect, it } from "vitest";
import {
  SKELETON_PATHS,
  type ScaffoldRow,
  deriveScaffoldStatus,
} from "../../src/lib/scaffold-status";

function stateOf(rows: readonly ScaffoldRow[], path: string): string | undefined {
  return rows.find((r) => r.path === path)?.state;
}

describe("deriveScaffoldStatus (POC-5a, honest — driven by real server facts)", () => {
  it("planning: every skeleton row is pending, none faked done/writing", () => {
    const d = deriveScaffoldStatus(SKELETON_PATHS, {
      phase: "planning",
      filesDone: [],
      filesPending: [...SKELETON_PATHS],
      detail: "",
    });
    expect(d.rows).toHaveLength(SKELETON_PATHS.length);
    expect(d.rows.every((r) => r.state === "pending")).toBe(true);
    expect(d.active).toBe(true);
    expect(d.complete).toBe(false);
    expect(d.failed).toBe(false);
    expect(d.phase).toBe("planning");
  });

  it("writing: done files are done, the FIRST not-done is writing, rest pending (one spinner)", () => {
    const d = deriveScaffoldStatus(SKELETON_PATHS, {
      phase: "writing",
      filesDone: ["README.md", "app.json"],
      filesPending: ["prompts/system.md", "rules/guardrails.md", "skills/main.md"],
      detail: "writing prompts/system.md",
    });
    expect(stateOf(d.rows, "README.md")).toBe("done");
    expect(stateOf(d.rows, "app.json")).toBe("done");
    // The first not-yet-done skeleton path is the single in-flight row.
    expect(stateOf(d.rows, "prompts/system.md")).toBe("writing");
    expect(stateOf(d.rows, "rules/guardrails.md")).toBe("pending");
    expect(stateOf(d.rows, "skills/main.md")).toBe("pending");
    // Exactly ONE writing row (honest — never many spinners).
    expect(d.rows.filter((r) => r.state === "writing")).toHaveLength(1);
    expect(d.active).toBe(true);
    expect(d.complete).toBe(false);
  });

  it("done: all skeleton rows are done, none writing, complete + not active", () => {
    const d = deriveScaffoldStatus(SKELETON_PATHS, {
      phase: "done",
      filesDone: [...SKELETON_PATHS],
      filesPending: [],
      detail: "",
    });
    expect(d.rows.every((r) => r.state === "done")).toBe(true);
    expect(d.complete).toBe(true);
    expect(d.active).toBe(false);
    expect(d.failed).toBe(false);
    expect(d.phase).toBe("done");
  });

  it("failed: partial files stay visible, the rest are pending (no fabricated done)", () => {
    const d = deriveScaffoldStatus(SKELETON_PATHS, {
      phase: "failed",
      filesDone: ["README.md"],
      filesPending: ["app.json", "prompts/system.md", "rules/guardrails.md", "skills/main.md"],
      detail: "the model returned an empty body",
    });
    expect(stateOf(d.rows, "README.md")).toBe("done");
    // No writing row once failed — the partials are honest, the rest pending.
    expect(d.rows.filter((r) => r.state === "writing")).toHaveLength(0);
    expect(stateOf(d.rows, "app.json")).toBe("pending");
    expect(d.failed).toBe(true);
    expect(d.active).toBe(false);
    expect(d.complete).toBe(false);
  });

  it("renders a row for every skeleton path even when the server reports unknown extras", () => {
    const d = deriveScaffoldStatus(SKELETON_PATHS, {
      phase: "writing",
      filesDone: ["README.md", "totally/unexpected.txt"],
      filesPending: [],
      detail: "",
    });
    // Always exactly the fixed skeleton set — extras don't add rows.
    expect(d.rows.map((r) => r.path)).toEqual([...SKELETON_PATHS]);
  });
});
