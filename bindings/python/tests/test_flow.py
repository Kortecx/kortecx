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


# --- Batch B: portable blueprint export/import (Flow delegates to Chain) ----------


def test_flow_to_blueprint_round_trips_to_build() -> None:
    from kortecx.chains import Chain

    f = flow(seed=3).step(topic="hi").then("write about it")
    bp = f.to_blueprint()
    assert bp["seed"] == 3
    assert bp["execution_mode"] == "frozen"
    assert [s["kind"] for s in bp["steps"]] == ["pure", "model"]
    # The exported artifact re-compiles to the IDENTICAL request as the flow's build().
    assert Chain.from_blueprint(bp) == f.build()


def test_flow_export_writes_a_reimportable_file(tmp_path) -> None:
    from kortecx.chains import Chain

    f = flow(seed=1).agent("a").then("b")
    path = tmp_path / "bp.json"
    f.export(path)
    assert Chain.from_blueprint_file(path) == f.build()


# --- V2a g2/g4: Run-from-handle, await-any wait, Agent.stream, Result.json --------


class _FakeClient:
    """A minimal stub for the Flow/Agent terminals — records which wait path runs."""

    def __init__(self) -> None:
        self.any_calls = 0
        self.term_calls = 0

    def run_chain(self, chain: object, *, wait: bool = False, timeout: float = 120.0) -> object:
        from kortecx.run import Run

        run = Run(self, b"\x01" * 16, b"", b"")  # empty terminal ⇒ await-any
        return self._await_any(run._instance, timeout) if wait else run

    def _await_any(self, instance: bytes, timeout: float) -> str:
        self.any_calls += 1
        return "ANY"

    def _await_terminal(self, instance: bytes, terminal: bytes, timeout: float, mode: str) -> str:
        self.term_calls += 1
        return "TERM"


def test_result_json_aliases_to_dict() -> None:
    from kortecx.run import Result

    r = Result(
        instance_id="aa", terminal_mote_id="bb", state="COMMITTED", result_ref="cc", payload=b"hi"
    )
    assert r.json() == r.to_dict()
    assert r.json(include_payload=False) == r.to_dict(include_payload=False)
    assert r.json()["state"] == "COMMITTED"


def test_flow_submit_returns_a_run() -> None:
    from kortecx.run import Run

    run = flow().agent("a").submit(client=_FakeClient())
    assert isinstance(run, Run)


def test_run_wait_with_no_terminal_uses_await_any() -> None:
    fc = _FakeClient()
    run = flow().agent("a").submit(client=fc)
    assert run.wait() == "ANY"
    assert fc.any_calls == 1 and fc.term_calls == 0


def test_flow_run_uses_explicit_client_and_default(monkeypatch: pytest.MonkeyPatch) -> None:
    from kortecx import defaults

    fc = _FakeClient()
    # explicit client
    out = flow().agent("a").run(wait=True, client=fc)
    assert out == "ANY"
    # zero-config default client
    monkeypatch.setattr(defaults, "default_client", lambda: fc)
    out2 = flow().agent("a").run(wait=True)
    assert out2 == "ANY"


def test_agent_stream_returns_a_run() -> None:
    from kortecx.run import Run

    run = Agent("hi").stream("task", client=_FakeClient())
    assert isinstance(run, Run)
