/**
 * `composeProposeGoal` — hand the planner the WHOLE brief.
 *
 * The New App form's propose button sent `goal` alone, while the name, the prompt (the
 * instruction the App actually runs) and the attached filenames sat on screen unused.
 * `ProposeWorkflowRequest` has exactly one field and the server interpolates it verbatim,
 * so composing on the client is the whole fix — no wire change.
 */

import { describe, expect, it } from "vitest";
import { composeProposeGoal } from "../../src/lib/app-capability-prompt";

describe("composeProposeGoal", () => {
  it("is a NO-OP for a bare goal", () => {
    // The common case must stay byte-identical to what the planner received before, so
    // adding composition cannot change behaviour for an author who only filled in a goal.
    expect(composeProposeGoal({ name: "", goal: "  summarize a changelog  " })).toBe(
      "summarize a changelog",
    );
  });

  it("carries the name, the instruction and the attached filenames", () => {
    const out = composeProposeGoal({
      name: "Release Notes Writer",
      goal: "turn a changelog into release notes",
      prompt: "Group by feature, then fixes. Always cite the PR number.",
      attachments: ["changelog.md", "style-guide.md"],
    });
    expect(out).toContain("App: Release Notes Writer");
    expect(out).toContain("Goal: turn a changelog into release notes");
    expect(out).toContain("Instruction it runs each time: Group by feature");
    expect(out).toContain("Context files it can read: changelog.md, style-guide.md");
  });

  it("omits empty fields rather than emitting blank labels", () => {
    const out = composeProposeGoal({ name: "Triage", goal: "triage support mail", prompt: "   " });
    expect(out).toBe("App: Triage\nGoal: triage support mail");
    expect(out).not.toContain("Instruction");
    expect(out).not.toContain("Context files");
  });

  it("drops blank attachment names", () => {
    const out = composeProposeGoal({
      name: "",
      goal: "g",
      attachments: ["  ", "real.csv"],
    });
    expect(out).toContain("Context files it can read: real.csv");
  });
});
