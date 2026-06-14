/** PR-4.1b run export: stable, self-describing JSON (lightweight + rich). */

import { describe, expect, it } from "vitest";
import { type RunBundle, exportRunFilename, exportRunJson } from "../../src/lib/export-run";
import type { RunRecord } from "../../src/lib/recent-runs";

const record: RunRecord = {
  instanceId: "ab".repeat(16),
  terminalMoteId: "cd".repeat(16),
  recipeFingerprint: "ef".repeat(16),
  handle: "kx/recipes/echo",
  startedAt: 1_700_000_000_000,
  args: '{"topic":"hi"}',
};

describe("exportRunFilename", () => {
  it("slugs safely (no path chars) and never empties", () => {
    expect(exportRunFilename("Incident / triage!", 42)).toBe("kortecx-run-incident-triage-42.json");
    expect(exportRunFilename("   ", 7)).toBe("kortecx-run-run-7.json");
  });
});

describe("exportRunJson", () => {
  it("emits the lightweight envelope from the record alone", () => {
    const doc = JSON.parse(exportRunJson(record, "My run"));
    expect(doc).toMatchObject({
      kind: "kortecx.run",
      version: 1,
      name: "My run",
      instance_id: "ab".repeat(16),
      terminal_mote_id: "cd".repeat(16),
      handle: "kx/recipes/echo",
      args: '{"topic":"hi"}',
      started_at: 1_700_000_000_000,
    });
    expect(doc.results).toBeUndefined();
  });

  it("includes the committed bundle when given one (rich export)", () => {
    const bundle: RunBundle = {
      currentSeq: 9,
      motes: [
        {
          moteId: "11".repeat(16),
          state: 5,
          ndClass: 0,
          committedSeq: 9,
          resultRef: "22".repeat(16),
          parents: [{ parentId: "33".repeat(16), edgeKind: "data", nonCascade: false }],
        },
      ],
      artifacts: [
        {
          moteId: "11".repeat(16),
          resultRef: "22".repeat(16),
          kind: "text",
          text: "hi",
          byteLength: 2,
        },
      ],
    };
    const doc = JSON.parse(exportRunJson(record, "My run", bundle));
    expect(doc.results.currentSeq).toBe(9);
    expect(doc.results.artifacts[0].text).toBe("hi");
    expect(doc.results.motes[0].parents[0].edgeKind).toBe("data");
  });
});
