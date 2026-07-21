/**
 * `connectedComponent` — pull ONE run's Motes out of a journal that holds every run.
 *
 * `GetProjection` is not run-scoped by design: one `kx serve` is one journal with ONE
 * `instance_id` shared by every Invoke, chat turn, scaffold and cron fire. So the run
 * view showed the whole workspace, and on a long-lived serve it crossed MAX_DAG_NODES
 * and silently degraded to a table for every run.
 */

import { describe, expect, it } from "vitest";
import { connectedComponent } from "../../src/components/dag/dag-graph";
import type { MoteVM } from "../../src/kx/use-projection";

function mote(id: string, parents: string[] = []): MoteVM {
  return {
    moteId: id,
    stateCode: 4,
    ndClass: 0,
    promotion: 0,
    resultRef: null,
    committedSeq: 1,
    anomaly: null,
    moteDefHash: "",
    parents: parents.map((p) => ({
      parentId: p,
      edgeKind: "data" as const,
      nonCascade: false,
    })),
  };
}

/** Two independent runs in one journal — exactly what a shared serve accumulates. */
const JOURNAL: MoteVM[] = [
  mote("a1"),
  mote("a2", ["a1"]),
  mote("a3", ["a2"]),
  mote("b1"),
  mote("b2", ["b1"]),
];

describe("connectedComponent", () => {
  it("returns only the anchor's own run", () => {
    const ids = connectedComponent(JOURNAL, "a2").map((m) => m.moteId);
    expect(ids).toEqual(["a1", "a2", "a3"]);
  });

  it("reaches DESCENDANTS, not just ancestors", () => {
    // The traversal must be undirected. Anchoring on the run's first Mote and walking
    // only `parents[]` upward would return `["a1"]` — dropping the answer step and its
    // artifacts, i.e. most of what the user opened the page to see.
    expect(connectedComponent(JOURNAL, "a1").map((m) => m.moteId)).toEqual(["a1", "a2", "a3"]);
  });

  it("returns EMPTY for an anchor that is not in the fold", () => {
    // A stale link, or an old server that sent no salt. Empty is the signal the caller
    // turns into an honest notice; silently returning everything would restore the bug.
    expect(connectedComponent(JOURNAL, "zz")).toEqual([]);
  });

  it("preserves the projection's own order", () => {
    const shuffled = [JOURNAL[2], JOURNAL[3], JOURNAL[0], JOURNAL[4], JOURNAL[1]] as MoteVM[];
    expect(connectedComponent(shuffled, "a3").map((m) => m.moteId)).toEqual(["a3", "a1", "a2"]);
  });

  it("ignores a dangling parent that has not surfaced yet", () => {
    // A child can appear a poll before its parent; the edge must not pull in a Mote that
    // is not present, nor throw.
    const early = [...JOURNAL, mote("c1", ["not-yet"])];
    expect(connectedComponent(early, "c1").map((m) => m.moteId)).toEqual(["c1"]);
  });

  it("handles a cycle without looping forever", () => {
    // The projection is a DAG, but this is untrusted server data folded client-side —
    // a visited set is the difference between a filter and a hung tab.
    const cyclic = [mote("x", ["y"]), mote("y", ["x"])];
    expect(
      connectedComponent(cyclic, "x")
        .map((m) => m.moteId)
        .sort(),
    ).toEqual(["x", "y"]);
  });

  it("is empty for an empty projection", () => {
    expect(connectedComponent([], "a1")).toEqual([]);
  });
});
