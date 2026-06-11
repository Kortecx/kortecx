"""The Blueprint builder — pure unit tests (no server). Maps the author-side DAG to
the wire (kinds -> enum, hex -> bytes, str -> utf-8). SN-8: never computes identity."""

from __future__ import annotations

from kortecx import BlueprintBuilder, EdgeInput, StepInput
from kortecx.v1 import coordinator_pb2 as c
from kortecx.v1 import gateway_pb2 as g


def test_two_step_data_dag_maps_to_the_request():
    b = BlueprintBuilder(7)
    root = b.add_step(StepInput(kind="pure", params={"topic": "hi"}))
    sink = b.add_step(StepInput(kind="pure"))
    b.add_edge(EdgeInput(parent=root, child=sink, edge="data"))

    # add_step returns the step index (the edge handle).
    assert (root, sink) == (0, 1)

    req = b.build()
    assert req.seed == 7
    assert len(req.steps) == 2
    assert req.steps[0].kind == g.WorkflowStepKind.WORKFLOW_STEP_KIND_PURE
    assert req.steps[0].params["topic"] == b"hi"
    assert len(req.edges) == 1
    assert req.edges[0].edge_kind == c.EdgeKind.EDGE_KIND_DATA
    assert req.execution_mode == g.WorkflowExecutionMode.WORKFLOW_EXECUTION_MODE_FROZEN


def test_model_step_control_edge_and_dynamic_mode():
    b = BlueprintBuilder().mode("dynamic")
    a = b.add_step(StepInput(kind="model", model_id="qwen3", prompt="summarize"))
    cc = b.add_step(StepInput(kind="pure"))
    b.add_edge(EdgeInput(parent=a, child=cc, edge="control", non_cascade=True))

    req = b.build()
    assert req.steps[0].kind == g.WorkflowStepKind.WORKFLOW_STEP_KIND_MODEL
    assert req.steps[0].model_id == "qwen3"
    assert req.steps[0].prompt == "summarize"
    assert req.edges[0].edge_kind == c.EdgeKind.EDGE_KIND_CONTROL
    assert req.edges[0].non_cascade is True
    assert req.execution_mode == g.WorkflowExecutionMode.WORKFLOW_EXECUTION_MODE_DYNAMIC
