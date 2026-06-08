/** Pure DAG-graph transform: edges from parents[], dangling-drop, topology hash. */

import { describe, expect, it } from "vitest";
import { buildEdges, topologyHash } from "../../src/components/dag/dag-graph";
import { toProjectionVM } from "../../src/kx/use-projection";
import {
  chainProjection,
  controlEdgeProjection,
  cycleProjection,
  diamondProjection,
  disconnectedProjection,
  fanInProjection,
  fanOutProjection,
  growsBetweenPolls,
  mote,
  nid,
  projection,
} from "../mocks/projection-fixtures";

const motesOf = (p: ReturnType<typeof projection>) => toProjectionVM(p).motes;

describe("buildEdges", () => {
  it("chain a→b→c yields parent→child edges", () => {
    const edges = buildEdges(motesOf(chainProjection(3)));
    expect(edges.map((e) => `${e.source.slice(-1)}>${e.target.slice(-1)}`)).toEqual(["0>1", "1>2"]);
  });

  it("diamond a→{b,c}→d yields four edges", () => {
    expect(buildEdges(motesOf(diamondProjection()))).toHaveLength(4);
  });

  it("fan-out / fan-in edge counts", () => {
    expect(buildEdges(motesOf(fanOutProjection(5)))).toHaveLength(5);
    expect(buildEdges(motesOf(fanInProjection(4)))).toHaveLength(4);
  });

  it("disconnected subgraphs keep their own edges", () => {
    expect(buildEdges(motesOf(disconnectedProjection()))).toHaveLength(2);
  });

  it("carries edge kind + non-cascade", () => {
    const edges = buildEdges(motesOf(controlEdgeProjection()));
    const byKind = edges.map((e) => `${e.edgeKind}${e.nonCascade ? "!" : ""}`).sort();
    expect(byKind).toEqual(["control", "control!", "data"]);
  });

  it("DROPS a dangling edge whose parent is absent at the frontier", () => {
    const orphan = projection([mote({ moteId: nid(5), parents: [{ parentId: nid(99) }] })]);
    expect(buildEdges(motesOf(orphan))).toEqual([]);
  });

  it("keeps both edges of a 2-cycle (defensive — no crash)", () => {
    expect(buildEdges(motesOf(cycleProjection()))).toHaveLength(2);
  });
});

describe("topologyHash", () => {
  it("is identical for the same topology with different STATE (no-thrash invariant)", () => {
    const [, grown, stateOnly] = growsBetweenPolls();
    // `grown` and `stateOnly` share ids+edges; only the children's state differs.
    expect(topologyHash(motesOf(stateOnly))).toBe(topologyHash(motesOf(grown)));
  });

  it("CHANGES when the topology grows (a dynamic child appears)", () => {
    const [rootOnly, grown] = growsBetweenPolls();
    expect(topologyHash(motesOf(grown))).not.toBe(topologyHash(motesOf(rootOnly)));
  });

  it("CHANGES when an edge's kind/cascade differs", () => {
    const dataOnly = projection([
      mote({ moteId: nid(0) }),
      mote({ moteId: nid(1), parents: [{ parentId: nid(0), edgeKind: "data" }] }),
    ]);
    const control = projection([
      mote({ moteId: nid(0) }),
      mote({ moteId: nid(1), parents: [{ parentId: nid(0), edgeKind: "control" }] }),
    ]);
    expect(topologyHash(motesOf(dataOnly))).not.toBe(topologyHash(motesOf(control)));
  });

  it("is order-independent (sorted ids + edges)", () => {
    const a = projection([mote({ moteId: nid(0) }), mote({ moteId: nid(1) })]);
    const b = projection([mote({ moteId: nid(1) }), mote({ moteId: nid(0) })]);
    expect(topologyHash(motesOf(a))).toBe(topologyHash(motesOf(b)));
  });
});
