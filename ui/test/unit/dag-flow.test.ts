/** Pure reactflow adapters: positioned nodes + styled edges from the projection. */

import { describe, expect, it } from "vitest";
import { buildFlowEdges, buildFlowNodes } from "../../src/components/dag/flow";
import type { BatchedContentVM } from "../../src/kx/use-content-batch";
import { toProjectionVM } from "../../src/kx/use-projection";
import { decodeContent } from "../../src/lib/content-decode";
import { diamondProjection, mote, nid, projection } from "../mocks/projection-fixtures";

const enc = (s: string) => new TextEncoder().encode(s);
function vm(ref: string, text: string, missing = false): BatchedContentVM {
  return {
    contentRef: ref,
    missing,
    truncated: false,
    fullSize: text.length,
    content: decodeContent(enc(text)),
  };
}

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

  it("without a results lookup, nodes carry no resolved content", () => {
    expect(nodes[0]?.data.resultContent).toBeUndefined();
    expect(nodes[0]?.data.resultMissing).toBe(false);
    expect(nodes[0]?.data.resultLoading).toBe(false);
  });
});

describe("buildFlowNodes — resolved results (D142.2)", () => {
  const refA = "11".repeat(32);
  const committed = toProjectionVM(
    projection([
      mote({ moteId: nid(100), resultRef: refA }),
      mote({ moteId: nid(101), resultRef: null }), // uncommitted
    ]),
  ).motes;
  const positions = new Map<string, { x: number; y: number }>();

  it("threads the resolved text onto a committed node", () => {
    const out = buildFlowNodes(committed, positions, {
      byRef: new Map([[refA, vm(refA, "resolved output")]]),
      loading: false,
    });
    expect(out[0]?.data.resultContent?.text).toBe("resolved output");
    expect(out[0]?.data.resultMissing).toBe(false);
    expect(out[0]?.data.resultLoading).toBe(false);
  });

  it("an uncommitted node never carries content or a loading flag", () => {
    const out = buildFlowNodes(committed, positions, { byRef: new Map(), loading: true });
    // node[1] has no resultRef → no content, and loading stays false for it
    expect(out[1]?.data.resultContent).toBeUndefined();
    expect(out[1]?.data.resultLoading).toBe(false);
    // node[0] HAS a ref but it isn't resolved yet → loading propagates
    expect(out[0]?.data.resultLoading).toBe(true);
    expect(out[0]?.data.resultContent).toBeUndefined();
  });

  it("propagates the uniform-empty (missing) verdict", () => {
    const out = buildFlowNodes(committed, positions, {
      byRef: new Map([[refA, vm(refA, "", true)]]),
      loading: false,
    });
    expect(out[0]?.data.resultMissing).toBe(true);
  });
});

describe("buildFlowEdges", () => {
  it("delegates to buildEdges + toRfEdge (diamond → 4 styled edges)", () => {
    const edges = buildFlowEdges(toProjectionVM(diamondProjection()).motes);
    expect(edges).toHaveLength(4);
    expect(edges[0]?.className).toContain("dag-edge");
  });
});
