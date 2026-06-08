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
});
