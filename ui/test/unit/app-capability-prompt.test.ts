/**
 * PR-G: the client-composed capability prompt baked into a new App's envelope.
 */

import { describe, expect, it } from "vitest";
import { CAPABILITY_PROMPT, composeCapabilityPrompt } from "../../src/lib/app-capability-prompt";

describe("composeCapabilityPrompt (PR-G)", () => {
  it("includes the base capability guidance (loop + runtime capabilities + honesty)", () => {
    const p = composeCapabilityPrompt("");
    expect(p).toContain(CAPABILITY_PROMPT);
    expect(p).toMatch(/reason/i);
    expect(p).toMatch(/tools/i);
    expect(p).toMatch(/connections/i);
    expect(p).toMatch(/datasets/i);
    expect(p).toMatch(/never fabricate/i);
  });

  it("appends the App's goal when provided", () => {
    const p = composeCapabilityPrompt("Summarize the changelog into release notes");
    expect(p).toContain("This App's goal");
    expect(p).toContain("Summarize the changelog into release notes");
  });

  it("lists attachment filenames when provided", () => {
    const p = composeCapabilityPrompt("goal", ["spec.md", "data.csv"]);
    expect(p).toContain("Attached context files");
    expect(p).toContain("spec.md");
    expect(p).toContain("data.csv");
  });

  it("omits the goal + attachment sections when empty (no dangling headers)", () => {
    const p = composeCapabilityPrompt("", []);
    expect(p).not.toContain("This App's goal");
    expect(p).not.toContain("Attached context files");
  });
});
