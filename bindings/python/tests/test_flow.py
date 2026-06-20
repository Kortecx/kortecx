"""The fluent Flow builder + first-class Agent — pure unit tests (no server).

A Flow is sugar over the operator AST, so the core assertion is PARITY: a fluent
chain lowers byte-identically to the equivalent operator/DSL chain. Plus Agent
frozen/dynamic-lane shape + zero-config resolution.
"""

from __future__ import annotations

import pytest

from kortecx.agent import Agent
from kortecx.chains import Chain, ChainError, model, pure
from kortecx.flow import Flow, flow


def test_flow_sequence_matches_operator_sugar() -> None:
    # .agent(...).then(...) == model() >> model()
    fluent = flow().agent("research", tools=["web-search"]).then("review").lowering()
    sugar = Chain.from_node(
        model(prompt="research", tools=["web-search"]) >> model(prompt="review")
    ).lowering()
    assert fluent == sugar
    # two model steps, one data edge, default (empty) model_id
    assert [s["kind"] for s in fluent["steps"]] == ["model", "model"]
    assert fluent["steps"][0]["model_id"] == ""
    assert fluent["steps"][0]["tool_contract"] == {"web-search": "1"}
    assert fluent["edges"] == [{"parent": 0, "child": 1, "edge": "data"}]


def test_flow_parallel_fans_out_and_in() -> None:
    # fan-out: a > [b & c]
    fan_out = flow().agent("a").parallel("b", "c").lowering()
    assert len(fan_out["steps"]) == 3
    assert sorted((e["parent"], e["child"]) for e in fan_out["edges"]) == [(0, 1), (0, 2)]
    # fan-in: [a & b] > c
    fan_in = flow().parallel("a", "b").then("c").lowering()
    assert sorted((e["parent"], e["child"]) for e in fan_in["edges"]) == [(0, 2), (1, 2)]


def test_flow_step_and_tool() -> None:
    lowered = flow().step(topic="hi").tool("echo", "1", n=3).lowering()
    assert lowered["steps"][0]["kind"] == "pure"
    assert lowered["steps"][0]["params"] == {"topic": "hi"}
    assert lowered["steps"][1]["kind"] == "tool"
    assert lowered["steps"][1]["tool_contract"] == {"echo": "1"}
    assert lowered["steps"][1]["params"] == {"kx.tool.args": '{"n":3}'}


def test_flow_then_accepts_a_task_and_subflow() -> None:
    sub = flow().agent("inner")
    chained = flow().agent("outer").then(pure(label="x")).then(sub).lowering()
    assert [s["kind"] for s in chained["steps"]] == ["model", "pure", "model"]
    assert chained["edges"] == [
        {"parent": 0, "child": 1, "edge": "data"},
        {"parent": 1, "child": 2, "edge": "data"},
    ]


def test_flow_context_and_seed() -> None:
    lowered = flow(seed=7).agent("a").context("team/ctx/spec").lowering()
    assert lowered["context_bundles"] == ["team/ctx/spec"]
    assert flow(seed=7).agent("a").to_chain()._seed == 7  # type: ignore[attr-defined]


def test_empty_flow_is_fail_closed() -> None:
    with pytest.raises(ChainError):
        flow().to_chain()
    with pytest.raises(ChainError):
        flow().parallel().agent("a")  # parallel with no branch


def test_agent_frozen_lane_is_a_single_agent_step() -> None:
    a = Agent("You are helpful.", tools=["echo"], reasoning="minimal")
    lowered = a.as_flow("do it").lowering()
    assert len(lowered["steps"]) == 1
    step = lowered["steps"][0]
    assert step["kind"] == "model"
    assert step["tool_contract"] == {"echo": "1"}
    assert step["params"] == {"reasoning": "minimal"}
    # the instruction is prepended to the task
    assert step["prompt"] == "You are helpful.\n\ndo it"


def test_agent_default_lane_is_frozen() -> None:
    assert Agent("x").dynamic is False
    assert Agent("x", dynamic=True).dynamic is True


def test_zero_config_resolution(monkeypatch: pytest.MonkeyPatch) -> None:
    from kortecx import defaults

    # env wins over the default; config is best-effort (skip the real file).
    monkeypatch.setattr(defaults, "_load_config", lambda: {})
    monkeypatch.setenv("KX_ENDPOINT", "http://example:1234")
    monkeypatch.setenv("KX_DEFAULT_MODEL", "kx-serve:envmodel")
    assert defaults.resolve_endpoint() == "http://example:1234"
    assert defaults.resolve_default_model() == "kx-serve:envmodel"
    # explicit beats env
    assert defaults.resolve_endpoint("http://explicit:9") == "http://explicit:9"
    # config provides a fallback when env is absent
    monkeypatch.delenv("KX_DEFAULT_MODEL")
    monkeypatch.setattr(defaults, "_load_config", lambda: {"default_model": "kx-serve:cfg"})
    assert defaults.resolve_default_model() == "kx-serve:cfg"


def test_flow_is_a_flow_instance() -> None:
    assert isinstance(flow(), Flow)
