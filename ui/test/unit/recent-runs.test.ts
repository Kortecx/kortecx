import { afterEach, describe, expect, it, vi } from "vitest";
import {
  type RunRecord,
  type ServerRun,
  clearRuns,
  loadRuns,
  mergeServerRuns,
  recordRun,
} from "../../src/lib/recent-runs";
import { runAnchor } from "../../src/lib/run-anchor";

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

  it("round-trips BOTH run anchors, so a reopened run is still scoped", () => {
    // The anchor is only knowable at submit time. If the salt does not survive the
    // localStorage round trip, closing and reopening a run silently widens its view back
    // to the whole journal — the failure is invisible, the page just shows other runs.
    recordRun(EP, {
      ...run("aa".repeat(8), 1),
      terminalMoteId: "7e".repeat(32),
      reactChainSalt: "5a".repeat(32),
    });
    const [reloaded] = loadRuns(EP);
    expect(reloaded?.terminalMoteId).toBe("7e".repeat(32));
    expect(reloaded?.reactChainSalt).toBe("5a".repeat(32));
    expect(runAnchor(reloaded ?? {})).toBe("5a".repeat(32));
  });

  it("a record written before the salt field existed still yields the terminal anchor", () => {
    // Forward/backward compatibility: `reactChainSalt` is optional, and an old record
    // must degrade to the terminal Mote rather than to "cannot scope".
    localStorage.setItem(
      `kortecx.ui.runs:${EP}`,
      JSON.stringify([
        {
          instanceId: "aa".repeat(8),
          terminalMoteId: "7e".repeat(32),
          recipeFingerprint: null,
          handle: null,
          startedAt: 1,
        },
      ]),
    );
    const [old] = loadRuns(EP);
    expect(old?.reactChainSalt).toBeUndefined();
    expect(runAnchor(old ?? {})).toBe("7e".repeat(32));
  });
});

describe("mergeServerRuns (UI-2 ListRuns merge)", () => {
  const srv = (instanceId: string, ms: number): ServerRun => ({
    instanceId,
    recipeFingerprint: "ff".repeat(32),
    registeredUnixMs: ms,
  });

  it("keeps local records and appends server-only instances, newest-first", () => {
    const local = [run("aa".repeat(8), 100)];
    const server = [srv("aa".repeat(8), 50), srv("bb".repeat(8), 200)];
    const merged = mergeServerRuns(local, server);
    // The shared instance keeps its LOCAL record (richer handle/terminal); the
    // server-only one is appended; sorted newest-first by startedAt.
    expect(merged.map((r) => r.instanceId)).toEqual(["bb".repeat(8), "aa".repeat(8)]);
    expect(merged[1]?.handle).toBe("kx/recipes/echo"); // local record preserved
    expect(merged[0]?.handle).toBeNull(); // server-only bare card
    expect(merged[0]?.startedAt).toBe(200);
  });

  it("no server runs → just the local list (UNIMPLEMENTED fallback)", () => {
    const local = [run("aa".repeat(8), 1)];
    expect(mergeServerRuns(local, [])).toEqual(local);
  });

  it("no local runs → server instances become bare cards", () => {
    const merged = mergeServerRuns([], [srv("cc".repeat(8), 9)]);
    expect(merged).toHaveLength(1);
    expect(merged[0]?.instanceId).toBe("cc".repeat(8));
    expect(merged[0]?.terminalMoteId).toBeNull();
    expect(merged[0]?.startedAt).toBe(9);
  });
});
