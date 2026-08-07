/**
 * MoteDag wiring + branching. The real reactflow canvas is covered by the browser
 * E2E (jsdom can't measure a viewport); here we stub `@xyflow/react` with a probe
 * that records the nodes/edges it receives, so MoteDag's logic is asserted
 * deterministically: counts, empty state, the >MAX table fallback, and the
 * no-relayout-on-state-only-poll invariant.
 */

import { render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

/** The sizes reactflow's store would publish after measuring. Mutable so a test can
 *  stand in for "the browser has measured the cards" — jsdom never will. */
const measured = vi.hoisted(() => ({
  current: [] as { id: string; measured?: { width?: number; height?: number } }[],
}));

/** The canvas size reactflow would publish. 0x0 = unmeasured (jsdom's real state). */
const container = vi.hoisted(() => ({ current: { width: 0, height: 0 } }));

vi.mock("@xyflow/react", () => ({
  ReactFlow: ({ nodes, edges }: { nodes: unknown[]; edges: unknown[] }) => (
    <div data-testid="rf" data-nodes={nodes.length} data-edges={edges.length} />
  ),
  ReactFlowProvider: ({ children }: { children: ReactNode }) => <>{children}</>,
  Background: () => null,
  Controls: () => null,
  MiniMap: () => null,
  useReactFlow: () => ({ fitView: () => {} }),
  useNodesInitialized: () => false,
  // The store the selector reads. jsdom measures nothing, so the default is an
  // EMPTY lookup — the same shape as a real first paint, which must fall back to the
  // declared box. A test opts in by populating `measured`.
  useStore: (sel: (s: unknown) => unknown) =>
    sel({
      nodeLookup: new Map(measured.current.map((n) => [n.id, n])),
      // No measured container in jsdom — layoutForContainer must then keep TB rather
      // than choose a direction from a zero.
      width: container.current.width,
      height: container.current.height,
    }),
  Handle: () => null,
  Position: { Top: "top", Bottom: "bottom" },
  MarkerType: { ArrowClosed: "arrowclosed", Arrow: "arrow" },
}));

import { MAX_DAG_NODES, MoteDag } from "../../../src/components/dag/MoteDag";
import * as layout from "../../../src/components/dag/layout";
import { toProjectionVM } from "../../../src/kx/use-projection";
import { connectedWrapper } from "../../mocks/harness";
import { makeMockClient } from "../../mocks/kx-client";
import {
  chainProjection,
  diamondProjection,
  fanInProjection,
  growsBetweenPolls,
  largeProjection,
  nid,
  projection,
  reactChainProjection,
  turnId,
} from "../../mocks/projection-fixtures";

const vm = (p: ReturnType<typeof projection>) => toProjectionVM(p);

// DagFlow batch-resolves committed results (run-scoped GetContentBatch), so the
// canvas needs a connected context + query client. Fixtures carry no result refs
// ⇒ the batch stays disabled; these tests assert nodes/edges/layout, not content.
const wrapper = connectedWrapper(makeMockClient().client);

// Every test starts UNMEASURED (a real first paint); the measured arm opts in.
afterEach(() => {
  measured.current = [];
  container.current = { width: 0, height: 0 };
});

describe("MoteDag", () => {
  it("renders the canvas with one node per Mote + one edge per parent (diamond)", () => {
    render(<MoteDag projection={vm(diamondProjection())} />, { wrapper });
    expect(screen.getByTestId("mote-dag")).toHaveAttribute("role", "img");
    const rf = screen.getByTestId("rf");
    expect(rf).toHaveAttribute("data-nodes", "4");
    expect(rf).toHaveAttribute("data-edges", "4");
  });

  it("empty projection → empty state, no canvas", () => {
    render(<MoteDag projection={vm(projection([]))} />, { wrapper });
    expect(screen.getByTestId("empty-state")).toBeInTheDocument();
    expect(screen.queryByTestId("mote-dag")).not.toBeInTheDocument();
  });

  it("renders the DAG at the node-count boundary (MAX)", () => {
    render(<MoteDag projection={vm(largeProjection(MAX_DAG_NODES))} />, { wrapper });
    expect(screen.getByTestId("mote-dag")).toBeInTheDocument();
    expect(screen.getByTestId("rf")).toHaveAttribute("data-nodes", String(MAX_DAG_NODES));
  });

  it("falls back to the table beyond MAX nodes", () => {
    render(<MoteDag projection={vm(largeProjection(MAX_DAG_NODES + 1))} />, { wrapper });
    expect(screen.getByTestId("dag-fallback")).toBeInTheDocument();
    expect(screen.getByTestId("mote-table")).toBeInTheDocument();
    expect(screen.queryByTestId("mote-dag")).not.toBeInTheDocument();
  });

  it("does NOT relayout on a state-only poll (no dagre thrash)", () => {
    const spy = vi.spyOn(layout, "layoutForContainer");
    const [, grown, stateOnly] = growsBetweenPolls();
    const { rerender } = render(<MoteDag projection={vm(grown)} />, { wrapper });
    const afterFirst = spy.mock.calls.length;
    expect(afterFirst).toBeGreaterThan(0);
    rerender(<MoteDag projection={vm(stateOnly)} />); // children flip COMMITTED — same topology
    expect(spy.mock.calls.length).toBe(afterFirst);
    spy.mockRestore();
  });

  it("relayouts when the topology grows (a dynamic child appears)", () => {
    const spy = vi.spyOn(layout, "layoutForContainer");
    const [rootOnly, grown] = growsBetweenPolls();
    const { rerender } = render(<MoteDag projection={vm(rootOnly)} />, { wrapper });
    const afterFirst = spy.mock.calls.length;
    rerender(<MoteDag projection={vm(grown)} />); // two children appear
    expect(spy.mock.calls.length).toBeGreaterThan(afterFirst);
    spy.mockRestore();
  });

  it("renders the swarm overview for a fan-in run (branch rows per branch)", () => {
    render(<MoteDag projection={vm(fanInProjection(3))} />, { wrapper });
    expect(screen.getByTestId("swarm-overview")).toBeInTheDocument();
    expect(screen.getByTestId("swarm-pattern-badge")).toBeInTheDocument();
    expect(screen.getAllByTestId("swarm-branch-row")).toHaveLength(3);
  });

  it("shows NO swarm overview for a plain linear run", () => {
    render(<MoteDag projection={vm(chainProjection(4))} />, { wrapper });
    expect(screen.queryByTestId("swarm-overview")).not.toBeInTheDocument();
  });
});

describe("MoteDag — an agentic run's chain", () => {
  /** A react-shaped run: N edge-free turn stars, plus the reader-derived roster. */
  const agentic = (turns: number) => ({
    ...vm(reactChainProjection({ turns })),
    agenticTurnIds: Array.from({ length: turns }, (_, k) => turnId(k)),
  });

  it("draws the turn chain: every turn + observation, with N-1 derived edges", () => {
    // 3 turns + 2 observations = 5 nodes; 2 durable turn→observation edges plus
    // 2 derived turn→turn edges = 4. Before the reader joined the turn record, the
    // canvas got 2 nodes and 1 edge however long the chain was.
    render(<MoteDag projection={agentic(3)} />, { wrapper });
    const rf = screen.getByTestId("rf");
    expect(rf.getAttribute("data-nodes")).toBe("5");
    expect(rf.getAttribute("data-edges")).toBe("4");
  });

  it("dagre SEES the derived edges (or the turns lay out as disconnected roots)", () => {
    // The failure this catches is subtle and worse than the original bug: the nodes all
    // render, but as a wide row of unconnected pairs, because turn Motes are parentless.
    const spy = vi.spyOn(layout, "layoutForContainer");
    render(<MoteDag projection={agentic(3)} />, { wrapper });
    const edgesPassed = spy.mock.calls[0]?.[1] ?? [];
    expect(edgesPassed.filter((e) => e.derived === true)).toHaveLength(2);
    spy.mockRestore();
  });

  it("explains the derived links, and only when there are some", () => {
    render(<MoteDag projection={agentic(3)} />, { wrapper });
    expect(screen.getByTestId("dag-derived-legend")).toBeInTheDocument();
  });

  it("a plain DAG run shows no derived-lineage legend", () => {
    // The honest-degrade side: a non-agentic run must look exactly as it did.
    render(<MoteDag projection={vm(diamondProjection())} />, { wrapper });
    expect(screen.queryByTestId("dag-derived-legend")).not.toBeInTheDocument();
  });

  it("a turn whose Mote has not landed adds no node and no edge", () => {
    // Mid-poll: the roster names 3 turns, the fold holds 2. The chain stays valid.
    const p = {
      ...vm(reactChainProjection({ turns: 3, presentTurns: 2 })),
      agenticTurnIds: [turnId(0), turnId(1), turnId(2)],
    };
    render(<MoteDag projection={p} />, { wrapper });
    const rf = screen.getByTestId("rf");
    expect(rf.getAttribute("data-nodes")).toBe("4"); // 2 turns + 2 observations
    expect(rf.getAttribute("data-edges")).toBe("3"); // 2 durable + 1 derived
  });

  it("a react-shaped run shows NO swarm overview", () => {
    // No Mote in the chain has ≥2 inbound Data parents, so nothing may read as a fan-in.
    render(<MoteDag projection={agentic(3)} />, { wrapper });
    expect(screen.queryByTestId("swarm-overview")).not.toBeInTheDocument();
  });
});

describe("MoteDag — the layout box comes from the rendered card", () => {
  /** Stand in for reactflow having measured the cards at `h` px tall. */
  const measure = (ids: readonly string[], h: number) => {
    measured.current = ids.map((id) => ({ id, measured: { width: 184, height: h } }));
  };

  it("hands dagre the MEASURED height, not the declared constant", () => {
    const spy = vi.spyOn(layout, "layoutForContainer");
    const ids = [0, 1, 2, 3].map((i) => nid(i));
    measure(ids, 155);
    render(<MoteDag projection={vm(chainProjection(4))} />, { wrapper });
    // The last call is the one that ran with the measurement in hand.
    const box = spy.mock.calls.at(-1)?.[2];
    expect(box?.nodeH).toBe(155);
    expect(box?.nodeW).toBe(184);
    for (const id of ids) {
      expect(box?.nodeHeights?.get(id)).toBe(155);
    }
    // The defect this pins: NODE_H is 72, and a card more than twice that tall was
    // laid out as if it were 72, so consecutive ranks were placed inside each other.
    expect(box?.nodeH).not.toBe(layout.NODE_H);
    spy.mockRestore();
  });

  it("falls back to the declared box while nothing is measured (first paint)", () => {
    const spy = vi.spyOn(layout, "layoutForContainer");
    render(<MoteDag projection={vm(chainProjection(4))} />, { wrapper });
    expect(spy.mock.calls.at(-1)?.[2]).toEqual({});
    spy.mockRestore();
  });

  it("relayouts when a card's measured height CHANGES (a row appears)", () => {
    const ids = [0, 1, 2, 3].map((i) => nid(i));
    measure(ids, 155);
    const { rerender } = render(<MoteDag projection={vm(chainProjection(4))} />, { wrapper });
    const spy = vi.spyOn(layout, "layoutForContainer");
    const before = spy.mock.calls.length;
    measure(ids, 260); // the card grows a row
    rerender(<MoteDag projection={vm(chainProjection(4))} />);
    expect(spy.mock.calls.length).toBeGreaterThan(before);
    expect(spy.mock.calls.at(-1)?.[2]?.nodeH).toBe(260);
    spy.mockRestore();
  });

  it("does NOT relayout when the measured sizes are unchanged", () => {
    const ids = [0, 1, 2, 3].map((i) => nid(i));
    measure(ids, 155);
    const { rerender } = render(<MoteDag projection={vm(chainProjection(4))} />, { wrapper });
    const spy = vi.spyOn(layout, "layoutForContainer");
    const before = spy.mock.calls.length;
    measure(ids, 155); // same sizes, new array identity — must not thrash dagre
    rerender(<MoteDag projection={vm(chainProjection(4))} />);
    expect(spy.mock.calls.length).toBe(before);
    spy.mockRestore();
  });
});
