/**
 * The reader-synthesised chain edges. These pin the frontier behaviour that a settled-run
 * test cannot see: the projection and the turn record are two reads of a moving journal,
 * so a turn's row can exist before its Mote does, and the drawn chain must stay valid
 * (and honest) at every intermediate state.
 */

import { describe, expect, it } from "vitest";
import { derivedChainEdges } from "../../src/components/dag/derived-lineage";
import { toProjectionVM } from "../../src/kx/use-projection";
import { obsId, reactChainProjection, turnId } from "../mocks/projection-fixtures";

const motesOf = (opts: Parameters<typeof reactChainProjection>[0]) =>
  toProjectionVM(reactChainProjection(opts)).motes;

describe("derivedChainEdges", () => {
  it("links consecutive turns, and marks every edge DERIVED", () => {
    const motes = motesOf({ turns: 3 });
    const edges = derivedChainEdges([turnId(0), turnId(1), turnId(2)], motes);
    expect(edges).toHaveLength(2);
    expect(edges.every((e) => e.derived === true)).toBe(true);
    expect(edges.map((e) => [e.source, e.target])).toEqual([
      [turnId(0), turnId(1)],
      [turnId(1), turnId(2)],
    ]);
  });

  it("uses an edge-id form that CANNOT collide with a durable parent->child id", () => {
    // Anything keyed on edge ids (the swarm branch highlight builds `->` ids) must never
    // pick up a synthesised edge by accident.
    const edges = derivedChainEdges([turnId(0), turnId(1)], motesOf({ turns: 2 }));
    expect(edges[0]?.id).toBe(`${turnId(0)}~>${turnId(1)}`);
    expect(edges[0]?.id).not.toContain("->");
  });

  it("DROPS a roster entry whose Mote is not at this frontier", () => {
    // Turn 2's row has arrived but its Mote has not. Drawing it would mean inventing a
    // state for a Mote we simply have not read yet.
    const motes = motesOf({ turns: 3, presentTurns: 2 });
    const edges = derivedChainEdges([turnId(0), turnId(1), turnId(2)], motes);
    expect(edges).toHaveLength(1);
    expect(edges.map((e) => e.target)).not.toContain(turnId(2));
  });

  it("BRIDGES a missing middle turn with ONE edge, never a dangling one", () => {
    // The roster is compressed before pairing, so a hole joins its neighbours directly
    // and everything downstream of the gap stays connected (and stays laid out).
    const motes = motesOf({ turns: 3, absentTurns: [1] });
    const edges = derivedChainEdges([turnId(0), turnId(1), turnId(2)], motes);
    expect(edges).toHaveLength(1);
    expect([edges[0]?.source, edges[0]?.target]).toEqual([turnId(0), turnId(2)]);
    const present = new Set(motes.map((m) => m.moteId));
    expect(edges.every((e) => present.has(e.source) && present.has(e.target))).toBe(true);
  });

  it("an empty or single-entry roster yields no edges", () => {
    const motes = motesOf({ turns: 2 });
    expect(derivedChainEdges([], motes)).toEqual([]);
    expect(derivedChainEdges([turnId(0)], motes)).toEqual([]);
  });

  it("a repeated roster entry never becomes a self-edge", () => {
    const motes = motesOf({ turns: 2 });
    const edges = derivedChainEdges([turnId(0), turnId(0), turnId(1)], motes);
    expect(edges.every((e) => e.source !== e.target)).toBe(true);
    expect(edges).toHaveLength(1);
  });

  it("does not disturb the durable star an observation already forms", () => {
    // The synthesis ADDS the turn order; the turn→observation Data edges keep coming
    // from `parents[]` and are untouched here.
    const motes = motesOf({ turns: 2 });
    const edges = derivedChainEdges([turnId(0), turnId(1)], motes);
    expect(edges.some((e) => e.source === obsId(0) || e.target === obsId(0))).toBe(false);
  });
});
