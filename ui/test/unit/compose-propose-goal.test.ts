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

  it("names the capabilities the App was given, as wishes", () => {
    // The create form grew tools/skills/integrations rails, and the planner was told
    // none of them — so it proposed steps for an App it believed had no capabilities,
    // and the attached tools only became real at run time in a DAG never shaped around
    // them. Names only, and never a credential value.
    const out = composeProposeGoal({
      name: "Triage Bot",
      goal: "triage inbound support mail",
      tools: ["mcp-echo/echo", "retrieve"],
      skills: ["classification"],
      connections: ["github/issues"],
    });
    expect(out).toContain("Tools it may request: mcp-echo/echo, retrieve");
    expect(out).toContain("Skills it carries: classification");
    expect(out).toContain("Integrations it can reach: github/issues");
  });

  it("a bare goal still composes to itself when no capability is attached", () => {
    // The invariant that keeps the common case byte-identical to what the planner
    // received before the rails existed — adding three optional inputs must not change
    // the prompt for an App that uses none of them.
    expect(composeProposeGoal({ name: "", goal: "just a goal" })).toBe("just a goal");
    expect(
      composeProposeGoal({
        name: "",
        goal: "just a goal",
        tools: ["  "],
        skills: [],
        connections: [],
      }),
    ).toBe("just a goal");
  });
});
