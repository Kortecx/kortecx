/** The Blueprint builder — pure, no server. Maps the author-side DAG to the wire
 *  (kinds → enum, hex → bytes, strings → utf-8). The builder never computes a
 *  MoteId/warrant (SN-8); the server compiles + admits. */

import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import { BlueprintBuilder } from "../src/blueprints.js";
import { EdgeKind } from "../src/gen/kortecx/v1/coordinator_pb.js";
import {
  SubmitWorkflowRequestSchema,
  WorkflowExecutionMode,
  WorkflowStepKind,
} from "../src/gen/kortecx/v1/gateway_pb.js";

describe("BlueprintBuilder", () => {
  it("maps a two-step DATA-edge DAG to the wire request", () => {
    const b = new BlueprintBuilder(7);
    const root = b.addStep({ kind: "pure", params: { topic: "hi" } });
    const sink = b.addStep({ kind: "pure" });
    b.addEdge({ parent: root, child: sink, edge: "data" });

    // addStep returns the step index (the edge handle).
    expect(root).toBe(0);
    expect(sink).toBe(1);

    const req = create(SubmitWorkflowRequestSchema, b.build());
    expect(req.seed).toBe(7);
    expect(req.steps).toHaveLength(2);
    const [s0] = req.steps;
    const [e0] = req.edges;
    expect(s0?.kind).toBe(WorkflowStepKind.PURE);
    expect(s0?.params.topic).toEqual(new TextEncoder().encode("hi"));
    expect(req.edges).toHaveLength(1);
    expect(e0?.edgeKind).toBe(EdgeKind.DATA);
    expect(req.executionMode).toBe(WorkflowExecutionMode.FROZEN);
  });

  it("maps model steps, control edges, and dynamic mode", () => {
    const b = new BlueprintBuilder().mode("dynamic");
    const a = b.addStep({ kind: "model", modelId: "qwen3", prompt: "summarize" });
    const c = b.addStep({ kind: "pure" });
    b.addEdge({ parent: a, child: c, edge: "control", nonCascade: true });

    const req = create(SubmitWorkflowRequestSchema, b.build());
    const [s0] = req.steps;
    const [e0] = req.edges;
    expect(s0?.kind).toBe(WorkflowStepKind.MODEL);
    expect(s0?.modelId).toBe("qwen3");
    expect(s0?.prompt).toBe("summarize");
    expect(e0?.edgeKind).toBe(EdgeKind.CONTROL);
    expect(e0?.nonCascade).toBe(true);
    expect(req.executionMode).toBe(WorkflowExecutionMode.DYNAMIC);
  });
});
