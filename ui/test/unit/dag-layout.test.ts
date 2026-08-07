/** Pure dagre layout: positions all nodes, top-down, cycle/empty tolerant. */

import { describe, expect, it } from "vitest";
import { buildEdges } from "../../src/components/dag/dag-graph";
import {
  fitScale,
  layoutExtent,
  layoutForContainer,
  layoutGraph,
} from "../../src/components/dag/layout";
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

/**
 * The graph fits the SPACE IT HAS. A run's shape and the viewport's shape are
 * independent, so the rank direction is chosen from both rather than assumed.
 */
describe("layoutForContainer — the layout answers to the container", () => {
  const chainOf = (n: number) => {
    const motes = toProjectionVM(chainProjection(n)).motes;
    return { ids: motes.map((m) => m.moteId), edges: buildEdges(motes) };
  };

  it("turns a long chain sideways in a WIDE, SHORT canvas", () => {
    const { ids, edges } = chainOf(8);
    const { direction } = layoutForContainer(ids, edges, {}, { width: 1600, height: 300 });
    expect(direction).toBe("LR");
  });

  it("keeps a long chain top-to-bottom in a TALL canvas", () => {
    const { ids, edges } = chainOf(8);
    const { direction } = layoutForContainer(ids, edges, {}, { width: 700, height: 1400 });
    expect(direction).toBe("TB");
  });

  it("positions every node whichever direction wins", () => {
    const { ids, edges } = chainOf(6);
    for (const c of [
      { width: 1600, height: 300 },
      { width: 600, height: 1200 },
    ]) {
      const { positions } = layoutForContainer(ids, edges, {}, c);
      expect(positions.size).toBe(ids.length);
      for (const id of ids) {
        expect(Number.isFinite(positions.get(id)?.x)).toBe(true);
        expect(Number.isFinite(positions.get(id)?.y)).toBe(true);
      }
    }
  });

  it("an UNMEASURED container never decides a direction (keeps TB)", () => {
    // First paint and headless renderers report 0x0. Choosing from a zero would flip
    // the picture on no information at all.
    const { ids, edges } = chainOf(8);
    expect(layoutForContainer(ids, edges, {}, { width: 0, height: 0 }).direction).toBe("TB");
  });

  it("does not turn on a MARGINAL difference (a resize must not flip the picture)", () => {
    // A near-square canvas with a near-square graph: LR may fit trivially better, and
    // turning the whole graph for that would be far more disorienting than the gain.
    const { ids, edges } = chainOf(2);
    expect(layoutForContainer(ids, edges, {}, { width: 800, height: 780 }).direction).toBe("TB");
  });

  it("the chosen layout uses the space: the winner's fit scale is the larger", () => {
    const { ids, edges } = chainOf(8);
    const container = { width: 1600, height: 300 };
    const { positions, direction } = layoutForContainer(ids, edges, {}, container);
    const other = layoutGraph(ids, edges, { direction: direction === "LR" ? "TB" : "LR" });
    expect(fitScale(layoutExtent(positions), container)).toBeGreaterThan(
      fitScale(layoutExtent(other), container),
    );
  });
});

describe("fitScale", () => {
  it("is the limiting ratio of container to extent", () => {
    expect(fitScale({ width: 200, height: 100 }, { width: 400, height: 400 })).toBe(2);
    expect(fitScale({ width: 100, height: 400 }, { width: 400, height: 400 })).toBe(1);
  });

  it("degrades to 0 rather than Infinity/NaN on a zero extent or container", () => {
    // A zero here means "not measured", and a scale of Infinity would silently win
    // every comparison it took part in.
    expect(fitScale({ width: 0, height: 0 }, { width: 400, height: 400 })).toBe(0);
    expect(fitScale({ width: 200, height: 100 }, { width: 0, height: 0 })).toBe(0);
  });
});
