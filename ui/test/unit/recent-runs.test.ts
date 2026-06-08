import { afterEach, describe, expect, it, vi } from "vitest";
import { type RunRecord, clearRuns, loadRuns, recordRun } from "../../src/lib/recent-runs";

const EP = "http://127.0.0.1:50151";
const OTHER = "http://127.0.0.1:60000";

function run(instanceId: string, startedAt = 1): RunRecord {
  return {
    instanceId,
    terminalMoteId: null,
    recipeFingerprint: null,
    handle: "kx/recipes/echo",
    startedAt,
  };
}

afterEach(() => {
  localStorage.clear();
  vi.restoreAllMocks();
});

describe("recent-runs", () => {
  it("starts empty", () => {
    expect(loadRuns(EP)).toEqual([]);
  });

  it("records newest-first and dedupes by instanceId", () => {
    recordRun(EP, run("a", 1));
    recordRun(EP, run("b", 2));
    const after = recordRun(EP, run("a", 3)); // re-record a → moves to front
    expect(after.map((r) => r.instanceId)).toEqual(["a", "b"]);
    expect(after[0]?.startedAt).toBe(3);
  });

  it("isolates histories per endpoint", () => {
    recordRun(EP, run("a"));
    recordRun(OTHER, run("z"));
    expect(loadRuns(EP).map((r) => r.instanceId)).toEqual(["a"]);
    expect(loadRuns(OTHER).map((r) => r.instanceId)).toEqual(["z"]);
  });

  it("caps the history at 50 entries", () => {
    for (let i = 0; i < 60; i++) {
      recordRun(EP, run(`r${i}`, i));
    }
    expect(loadRuns(EP)).toHaveLength(50);
  });

  it("clearRuns empties the endpoint history", () => {
    recordRun(EP, run("a"));
    clearRuns(EP);
    expect(loadRuns(EP)).toEqual([]);
  });

  it("corrupt store → empty list (no throw)", () => {
    localStorage.setItem(`kortecx.ui.runs:${EP}`, "{bad");
    expect(loadRuns(EP)).toEqual([]);
  });
});
