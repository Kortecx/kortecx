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

describe("buildFlowNodes — an OBSERVATION names its turn, not its hash", () => {
  // The turn-label fix covered TURN nodes only. The observation hanging off a turn —
  // the Mote that actually fired the tool — still read `bca492fe…a2b3` with a
  // `WORLD_MUTATING` badge: a content hash and a determinism-class enum, which is
  // machinery rather than what happened.
  const turnId = nid(901);
  const obsId = nid(902);
  const turnLabels = new Map([
    [turnId, { turn: 0, branch: "tool", toolId: "mcp-echo/echo", toolVersion: "1" }],
  ]);
  const motes = toProjectionVM(
    projection([
      mote({ moteId: turnId }),
      mote({ moteId: obsId, parents: [{ parentId: turnId }] }),
    ]),
  ).motes;

  it("carries the parent turn's label onto the observation", () => {
    const out = buildFlowNodes(motes, new Map(), undefined, undefined, undefined, turnLabels);
    const obs = out.find((n) => n.id === obsId);
    expect(obs?.data.observationOf).toMatchObject({ turn: 0, toolId: "mcp-echo/echo" });
  });

  it("does NOT label the turn itself as its own observation", () => {
    const out = buildFlowNodes(motes, new Map(), undefined, undefined, undefined, turnLabels);
    const turn = out.find((n) => n.id === turnId);
    expect(turn?.data.turnLabel).toBeDefined();
    expect(turn?.data.observationOf).toBeUndefined();
  });

  // ⚠ THE CONTROL. Without it this would pass just as well against a version that
  // labelled every child of anything — which would put a turn number on ordinary DAG
  // steps that have nothing to do with the agentic chain.
  it("leaves a non-agentic child alone", () => {
    const parentId = nid(903);
    const childId = nid(904);
    const plain = toProjectionVM(
      projection([mote({ moteId: parentId }), mote({ moteId: childId, parents: [{ parentId }] })]),
    ).motes;
    const out = buildFlowNodes(plain, new Map(), undefined, undefined, undefined, turnLabels);
    expect(out.find((n) => n.id === childId)?.data.observationOf).toBeUndefined();
  });

  // A fan-in has several parents and is not an observation of any one of them.
  it("leaves a multi-parent Mote alone", () => {
    const otherId = nid(905);
    const gatherId = nid(906);
    const fan = toProjectionVM(
      projection([
        mote({ moteId: turnId }),
        mote({ moteId: otherId }),
        mote({ moteId: gatherId, parents: [{ parentId: turnId }, { parentId: otherId }] }),
      ]),
    ).motes;
    const out = buildFlowNodes(fan, new Map(), undefined, undefined, undefined, turnLabels);
    expect(out.find((n) => n.id === gatherId)?.data.observationOf).toBeUndefined();
  });
});

describe("an observation must not impersonate its turn", () => {
  // ⚠ A LIVE RUN CAUGHT THIS, and nothing cheaper did. Labelling the observation
  // `Turn N` under the SAME `dag-node-turn` testid put two cards on the canvas
  // answering to the same turn: the graph reported
  //   Turn 1 -> ["MCP", "result of mcp-echo/echo@1"]
  // while the Timeline reported
  //   Turn 1 -> ["MCP", "mcp-echo/echo@1"]
  // and the W3′ guarantee that the two surfaces say the SAME words about a turn
  // broke — the collector keyed by turn number and kept whichever node came last.
  //
  // The unit tests below it all passed, because they asserted `observationOf` was
  // POPULATED, never what the two nodes looked like SIDE BY SIDE. A projection can
  // be perfectly correct and still render an ambiguous surface.
  const turnId = nid(911);
  const obsId = nid(912);
  const turnLabels = new Map([
    [turnId, { turn: 1, branch: "tool", toolId: "mcp-echo/echo", toolVersion: "1" }],
  ]);

  it("keeps exactly ONE node claiming to BE a given turn", () => {
    const motes = toProjectionVM(
      projection([
        mote({ moteId: turnId }),
        mote({ moteId: obsId, parents: [{ parentId: turnId }] }),
      ]),
    ).motes;
    const out = buildFlowNodes(motes, new Map(), undefined, undefined, undefined, turnLabels);
    // The turn is the turn; the observation is derived FROM it. Exactly one of each,
    // and they are never the same node.
    const turns = out.filter((n) => n.data.turnLabel !== undefined);
    const observations = out.filter((n) => n.data.observationOf !== undefined);
    expect(turns).toHaveLength(1);
    expect(observations).toHaveLength(1);
    expect(turns[0]?.id).not.toBe(observations[0]?.id);
    // Both name turn 1 — that is the point of the fix — but only one IS turn 1.
    expect(turns[0]?.data.turnLabel?.turn).toBe(1);
    expect(observations[0]?.data.observationOf?.turn).toBe(1);
  });
});
