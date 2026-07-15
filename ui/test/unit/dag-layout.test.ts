/** Pure dagre layout: positions all nodes, top-down, cycle/empty tolerant. */

import { describe, expect, it } from "vitest";
import { buildEdges } from "../../src/components/dag/dag-graph";
import { layoutGraph } from "../../src/components/dag/layout";
import { toProjectionVM } from "../../src/kx/use-projection";
import {
  chainProjection,
  cycleProjection,
  diamondProjection,
  mote,
  nid,
  projection,
} from "../mocks/projection-fixtures";

const layoutOf = (p: ReturnType<typeof projection>) => {
  const motes = toProjectionVM(p).motes;
  return layoutGraph(
    motes.map((m) => m.moteId),
    buildEdges(motes),
  );
};

/** The custom node box (the App Lineage cards) — the existing callers pass none. */
describe("layoutGraph node box", () => {
  const chain = () => {
    const motes = toProjectionVM(chainProjection(4)).motes;
    return { ids: motes.map((m) => m.moteId), edges: buildEdges(motes) };
  };

  it("is byte-identical to the default when no box is passed (the shared callers)", () => {
    const { ids, edges } = chain();
    // MoteDag / the blueprint builder call the 2-arg form; adding the box parameter
    // must not move a single node for them.
    expect([...layoutGraph(ids, edges).entries()]).toEqual([
      ...layoutGraph(ids, edges, {}).entries(),
    ]);
  });

  it("positions against the box it is given, not the default footprint", () => {
    const { ids, edges } = chain();
    const dflt = layoutGraph(ids, edges);
    const tall = layoutGraph(ids, edges, { nodeW: 248, nodeH: 124 });
    const spanOf = (m: Map<string, { x: number; y: number }>) => {
      const ys = [...m.values()].map((p) => p.y);
      return Math.max(...ys) - Math.min(...ys);
    };
    // A taller card must rank further apart, or the cards would overlap.
    expect(spanOf(tall)).toBeGreaterThan(spanOf(dflt));
  });

  it("honours a partial box (one dimension overridden)", () => {
    const { ids, edges } = chain();
    const pos = layoutGraph(ids, edges, { nodeH: 124 });
    for (const id of ids) {
      expect(pos.get(id)).toMatchObject({ x: expect.any(Number), y: expect.any(Number) });
    }
  });
});

describe("layoutGraph", () => {
  it("returns a position for every node", () => {
    const pos = layoutOf(diamondProjection());
    expect(pos.size).toBe(4);
    for (let i = 0; i < 4; i++) {
      expect(pos.get(nid(i))).toMatchObject({ x: expect.any(Number), y: expect.any(Number) });
    }
  });

  it("empty graph → empty positions", () => {
    expect(layoutGraph([], []).size).toBe(0);
  });

  it("lays a chain out top-to-bottom (child below parent)", () => {
    const pos = layoutOf(chainProjection(3));
    const y = (i: number) => pos.get(nid(i))?.y ?? 0;
    expect(y(1)).toBeGreaterThan(y(0));
    expect(y(2)).toBeGreaterThan(y(1));
  });

  it("a 2-cycle is laid out without hanging (positions both nodes)", () => {
    const pos = layoutOf(cycleProjection());
    expect(pos.size).toBe(2);
    expect(pos.get(nid(0))).toBeDefined();
    expect(pos.get(nid(1))).toBeDefined();
  });

  it("a single root has a finite position", () => {
    const pos = layoutOf(projection([mote({ moteId: nid(0) })]));
    expect(Number.isFinite(pos.get(nid(0))?.x)).toBe(true);
    expect(Number.isFinite(pos.get(nid(0))?.y)).toBe(true);
  });
});
