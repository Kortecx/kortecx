/** Pure reactflow adapters: positioned nodes + styled edges from the projection. */

import { describe, expect, it } from "vitest";
import { buildFlowEdges, buildFlowNodes } from "../../src/components/dag/flow";
import { toProjectionVM } from "../../src/kx/use-projection";
import { diamondProjection, nid } from "../mocks/projection-fixtures";

describe("buildFlowNodes", () => {
  const motes = toProjectionVM(diamondProjection()).motes;
  const positions = new Map(motes.map((m, i) => [m.moteId, { x: i * 10, y: i * 20 }]));
  const nodes = buildFlowNodes(motes, positions);

  it("one node per mote, typed + positioned + non-draggable", () => {
    expect(nodes).toHaveLength(4);
    expect(nodes[0]?.type).toBe("mote");
    expect(nodes[0]?.draggable).toBe(false);
    expect(nodes[1]?.position).toEqual({ x: 10, y: 20 });
  });

  it("carries the mote VM as node data", () => {
    expect(nodes[0]?.data.mote.moteId).toBe(nid(0));
  });

  it("a node with no layout position falls back to the origin", () => {
    const fallback = buildFlowNodes(motes, new Map());
    expect(fallback[0]?.position).toEqual({ x: 0, y: 0 });
  });
});

describe("buildFlowEdges", () => {
  it("delegates to buildEdges + toRfEdge (diamond → 4 styled edges)", () => {
    const edges = buildFlowEdges(toProjectionVM(diamondProjection()).motes);
    expect(edges).toHaveLength(4);
    expect(edges[0]?.className).toContain("dag-edge");
  });
});
