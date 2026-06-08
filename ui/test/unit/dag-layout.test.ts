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
