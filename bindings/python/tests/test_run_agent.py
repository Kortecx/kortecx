"""run_agent — the embeddable agent-runner (PR-9c-1). Pure unit tests (no server).

A thin wrapper over ``invoke("kx/recipes/react")`` + ``list_react_turns``; the tests
stub the client and assert the assembled :class:`AgentResult` (answer + audited
actions), the ``wait=False`` Run path, prompt-folding of ``inputs``, and the
zero-config default-client resolution.
"""

from __future__ import annotations

import pytest

from kortecx.agent_result import AgentResult, AuditedAction, assemble_actions
from kortecx.react import ReactTurn
from kortecx.run import Result, Run
from kortecx.run_agent import _args, _fold_inputs, run_agent


def _turn(turn: int, branch: str, tool_id: str = "", tool_version: str = "") -> ReactTurn:
    return ReactTurn(
        turn=turn,
        turn_mote_id="aa",
        instance_id="bb",
        model_id="m",
        branch=branch,
        tool_id=tool_id,
        tool_version=tool_version,
        max_turns=8,
        max_tool_calls=6,
        seq=turn,
    )


class _FakeTurnPage:
    def __init__(self, turns: list) -> None:
        self.turns = turns
        self.has_more = False


class _FakeClient:
    """Records the invoke call + returns a canned Result + react-turn page."""

    def __init__(self, *, turns: list | None = None, payload: bytes = b"the answer") -> None:
        self.turns = turns or []
        self.payload = payload
        self.invoke_calls: list[dict] = []

    def invoke(self, handle, args, *, context=None, wait=False, timeout=120.0):
        self.invoke_calls.append({"handle": handle, "args": args, "context": context, "wait": wait})
        if not wait:
            return Run(self, b"\x01" * 16, b"\x02" * 32, b"")
        return Result(
            instance_id="ab" * 8,
            terminal_mote_id="cd" * 32,
            state="COMMITTED",
            result_ref="ef" * 32,
            payload=self.payload,
            react_chain_salt="5a" * 32,
        )

    def list_react_turns(self, *, instance_id=None, step_salt=None, limit=None):
        # PR-R1: the runner scopes the action fetch to the invocation's chain.
        self.list_calls = getattr(self, "list_calls", [])
        self.list_calls.append({"instance_id": instance_id, "step_salt": step_salt})
        return _FakeTurnPage(self.turns)


def test_assemble_actions_filters_tool_turns_in_order() -> None:
    turns = [
        _turn(2, "tool", "fs-list", "1"),
        _turn(0, "pending"),
        _turn(1, "tool", "mcp-echo/echo", "1"),
        _turn(3, "answer"),
    ]
    actions = assemble_actions(turns)
    assert [a.turn for a in actions] == [1, 2]  # tool turns only, sorted by turn
    assert actions[0] == AuditedAction("mcp-echo/echo", "1", 1)


def test_run_agent_assembles_answer_and_actions() -> None:
    fc = _FakeClient(
        turns=[_turn(0, "tool", "mcp-echo/echo", "1"), _turn(1, "answer")],
        payload=b"pong",
    )
    out = run_agent("echo pong", client=fc)
    assert isinstance(out, AgentResult)
    assert out.answer == "pong"
    assert out.answer_bytes == b"pong"
    assert [a.tool_id for a in out.actions] == ["mcp-echo/echo"]
    assert out.run_handle == out.instance_id != ""
    assert out.ok
    # the steered react recipe was invoked with the bounded-loop budget
    call = fc.invoke_calls[0]
    assert call["handle"] == "kx/recipes/react"
    assert call["args"]["max_turns"] == 8 and call["args"]["max_tool_calls"] == 20
    assert call["args"]["instruction"] == "echo pong"


def test_run_agent_no_wait_returns_run() -> None:
    fc = _FakeClient()
    out = run_agent("do it", wait=False, client=fc)
    assert isinstance(out, Run)
    assert fc.invoke_calls[0]["wait"] is False


def test_run_agent_folds_inputs_into_prompt() -> None:
    fc = _FakeClient(turns=[_turn(0, "answer")])
    run_agent("summarize", inputs={"url": "x", "lang": "en"}, client=fc)
    instr = fc.invoke_calls[0]["args"]["instruction"]
    assert instr.startswith("summarize")
    assert "- url: x" in instr and "- lang: en" in instr


def test_run_agent_passes_context() -> None:
    fc = _FakeClient(turns=[_turn(0, "answer")])
    run_agent("g", context=["team/ctx/spec"], client=fc)
    assert fc.invoke_calls[0]["context"] == ["team/ctx/spec"]


def test_agent_result_json_shape() -> None:
    r = AgentResult(
        answer="hi",
        answer_bytes=b"hi",
        actions=[AuditedAction("mcp-echo/echo", "1", 0)],
        run_handle="ab",
        instance_id="ab",
    )
    j = r.json()
    assert j == r.to_dict()
    assert j["answer"] == "hi"
    assert j["instance_id"] == "ab"
    assert j["actions"] == [
        {"tool_id": "mcp-echo/echo", "tool_version": "1", "turn": 0, "call_index": 0}
    ]


def test_run_agent_zero_config_default_client(monkeypatch: pytest.MonkeyPatch) -> None:
    from kortecx import defaults

    fc = _FakeClient(turns=[_turn(0, "answer")], payload=b"ok")
    monkeypatch.setattr(defaults, "default_client", lambda: fc)
    out = run_agent("g")
    assert isinstance(out, AgentResult)
    assert out.answer == "ok"


def test_fold_inputs_noop_without_inputs() -> None:
    assert _fold_inputs("g", None) == "g"
    assert _args("g", None)["instruction"] == "g"
