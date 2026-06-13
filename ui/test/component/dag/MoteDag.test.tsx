/**
 * MoteDag wiring + branching. The real reactflow canvas is covered by the browser
 * E2E (jsdom can't measure a viewport); here we stub `@xyflow/react` with a probe
 * that records the nodes/edges it receives, so MoteDag's logic is asserted
 * deterministically: counts, empty state, the >MAX table fallback, and the
 * no-relayout-on-state-only-poll invariant.
 */

import { render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";

vi.mock("@xyflow/react", () => ({
  ReactFlow: ({ nodes, edges }: { nodes: unknown[]; edges: unknown[] }) => (
    <div data-testid="rf" data-nodes={nodes.length} data-edges={edges.length} />
  ),
  ReactFlowProvider: ({ children }: { children: ReactNode }) => <>{children}</>,
  Background: () => null,
  Controls: () => null,
  MiniMap: () => null,
  useReactFlow: () => ({ fitView: () => {} }),
  Handle: () => null,
  Position: { Top: "top", Bottom: "bottom" },
  MarkerType: { ArrowClosed: "arrowclosed" },
}));

import { MAX_DAG_NODES, MoteDag } from "../../../src/components/dag/MoteDag";
import * as layout from "../../../src/components/dag/layout";
import { toProjectionVM } from "../../../src/kx/use-projection";
import { connectedWrapper } from "../../mocks/harness";
import { makeMockClient } from "../../mocks/kx-client";
import {
  diamondProjection,
  growsBetweenPolls,
  largeProjection,
  projection,
} from "../../mocks/projection-fixtures";

const vm = (p: ReturnType<typeof projection>) => toProjectionVM(p);

// DagFlow batch-resolves committed results (run-scoped GetContentBatch), so the
// canvas needs a connected context + query client. Fixtures carry no result refs
// ⇒ the batch stays disabled; these tests assert nodes/edges/layout, not content.
const wrapper = connectedWrapper(makeMockClient().client);

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
    const spy = vi.spyOn(layout, "layoutGraph");
    const [, grown, stateOnly] = growsBetweenPolls();
    const { rerender } = render(<MoteDag projection={vm(grown)} />, { wrapper });
    const afterFirst = spy.mock.calls.length;
    expect(afterFirst).toBeGreaterThan(0);
    rerender(<MoteDag projection={vm(stateOnly)} />); // children flip COMMITTED — same topology
    expect(spy.mock.calls.length).toBe(afterFirst);
    spy.mockRestore();
  });

  it("relayouts when the topology grows (a dynamic child appears)", () => {
    const spy = vi.spyOn(layout, "layoutGraph");
    const [rootOnly, grown] = growsBetweenPolls();
    const { rerender } = render(<MoteDag projection={vm(rootOnly)} />, { wrapper });
    const afterFirst = spy.mock.calls.length;
    rerender(<MoteDag projection={vm(grown)} />); // two children appear
    expect(spy.mock.calls.length).toBeGreaterThan(afterFirst);
    spy.mockRestore();
  });
});
