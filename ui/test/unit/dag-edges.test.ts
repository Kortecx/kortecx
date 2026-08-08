/** Pure DAG-edge → reactflow-edge visual mapping. */

import { describe, expect, it } from "vitest";
import { buildEdges } from "../../src/components/dag/dag-graph";
import { toRfEdge } from "../../src/components/dag/edges";
import { toProjectionVM } from "../../src/kx/use-projection";
import { controlEdgeProjection, nid } from "../mocks/projection-fixtures";

describe("toRfEdge", () => {
  const edges = buildEdges(toProjectionVM(controlEdgeProjection()).motes);
  const by = (kind: string, nonCascade: boolean) => {
    const e = edges.find((x) => x.edgeKind === kind && x.nonCascade === nonCascade);
    if (e === undefined) {
      throw new Error(`no ${kind}${nonCascade ? " non-cascade" : ""} edge in fixture`);
    }
    return toRfEdge(e);
  };

  it("DATA edge: solid, full opacity, data class", () => {
    const rf = by("data", false);
    expect(rf.className).toContain("dag-edge--data");
    expect(rf.style?.strokeDasharray).toBeUndefined();
    expect(rf.style?.opacity).toBe(0.85);
  });

  it("CONTROL edge: dashed, control class", () => {
    const rf = by("control", false);
    expect(rf.className).toContain("dag-edge--control");
    expect(rf.style?.strokeDasharray).toBe("5 4");
  });

  it("non-cascade CONTROL edge: dimmed + noncascade class", () => {
    const rf = by("control", true);
    expect(rf.className).toContain("dag-edge--noncascade");
    expect(rf.style?.opacity).toBe(0.4);
  });

  it("preserves id/source/target + carries an arrow marker", () => {
    const rf = by("data", false);
    expect(rf.source).toBe(nid(0));
    expect(rf.target).toBe(nid(3));
    expect(rf.id).toContain("->");
    expect(rf.markerEnd).toBeTruthy();
  });

  it("PR-B: a swarm branch edge adds dag-edge--branch; the default path is unchanged", () => {
    const dataEdge = edges.find((x) => x.edgeKind === "data" && !x.nonCascade);
    if (dataEdge === undefined) {
      throw new Error("no data edge in fixture");
    }
    expect(toRfEdge(dataEdge).className).not.toContain("dag-edge--branch");
    expect(toRfEdge(dataEdge, { branch: true }).className).toContain("dag-edge--branch");
  });
});

describe("toRfEdge — a DERIVED edge", () => {
  const base = {
    id: "a~>b",
    source: "a",
    target: "b",
    edgeKind: "control" as const,
    nonCascade: false,
  };

  it("is dashed, class-marked, and carries an OPEN arrow head", () => {
    // Three non-colour signals, because a synthesised link must never be mistakable for
    // a recorded parent — and colour alone would fail the dual-theme bar anyway.
    const e = toRfEdge({ ...base, derived: true });
    expect(e.className).toContain("dag-edge--derived");
    expect(e.markerEnd).toMatchObject({ type: "arrow" });
    expect(e.data?.derived).toBe(true);

    // It is DASHED — asserted as a property rather than as the exact pattern. The
    // pattern was `2 5`, two parts ink to five of gap, and at default zoom that read
    // as almost nothing on the canvas; pinning the literal made the legibility defect
    // look like the specification. What must hold is that a dash exists and that
    // enough of it is ink to see.
    const dash = String(e.style?.strokeDasharray ?? "");
    expect(dash, "a derived edge must be dashed").toMatch(/^\d+\s+\d+$/);
    const [on = 0, off = 0] = dash.split(/\s+/).map(Number);
    expect(
      on,
      "the dash must carry as much ink as gap, or it disappears at 1x",
    ).toBeGreaterThanOrEqual(off);
  });

  it("leaves the durable path untouched — no derived class, closed head", () => {
    const e = toRfEdge({ ...base, edgeKind: "data" });
    expect(e.className).not.toContain("dag-edge--derived");
    expect(e.style?.strokeDasharray).toBeUndefined();
    expect(e.markerEnd).toMatchObject({ type: "arrowclosed" });
    expect(e.data?.derived).toBe(false);
  });

  it("a durable CONTROL edge keeps its own dash, distinct from a derived one", () => {
    expect(toRfEdge({ ...base }).style?.strokeDasharray).toBe("5 4");
  });
});
