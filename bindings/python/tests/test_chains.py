"""The Chains DSL — pure unit tests (no server).

The heart is the GOLDEN-CORPUS parity gate: every case in
``tests/golden/chains/corpus.json`` is parsed via the string DSL; success cases
must lower deep-equal to ``expect``, error cases must raise the matching error
class. A handful of operator-sugar tests assert ``>>`` / ``&`` / ``|`` lower
IDENTICALLY to the string form (the two front doors, one lowering).
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Dict

import pytest

from kortecx.chains import (
    AgenticStepError,
    Chain,
    ChainCycleError,
    ChainError,
    ChainParseError,
    Task,
    UnknownHandleError,
    chain,
    model,
    pure,
    tool,
)

# The corpus is the cross-surface contract: repo-root/tests/golden/chains. From
# this file (bindings/python/tests/) the repo root is parents[3].
_CORPUS_PATH = Path(__file__).resolve().parents[3] / "tests" / "golden" / "chains" / "corpus.json"

# error class string -> the exception we expect
_ERROR_CLASSES = {
    "parse": ChainParseError,
    "unknown_handle": UnknownHandleError,
    "cycle": ChainCycleError,
    "agentic_non_model": AgenticStepError,
}


def _load_corpus() -> list:
    return json.loads(_CORPUS_PATH.read_text())


def _task_from_spec(spec: Dict[str, object]) -> Task:
    """Build a :class:`Task` from a corpus task spec (``{kind, model_id?, prompt?,
    params?}`` for pure/model; ``{kind:"tool", tool_contract, args?}`` for tool)."""
    kind = spec["kind"]
    params = spec.get("params") or {}
    if kind == "model":
        return model(
            str(spec.get("model_id", "")),
            str(spec.get("prompt", "")),
            tools=spec.get("tool_contract"),
            max_turns=spec.get("max_turns"),
            max_tool_calls=spec.get("max_tool_calls"),
            **params,
        )
    if kind == "pure":
        return pure(**params)
    if kind == "tool":
        contract = spec.get("tool_contract") or {}
        (tool_id, tool_version) = next(iter(contract.items()))
        args = spec.get("args") or {}
        return tool(str(tool_id), str(tool_version), **args)
    raise AssertionError(f"unsupported corpus task kind {kind!r}")


def _tasks_from_spec(specs: Dict[str, Dict[str, object]]) -> Dict[str, Task]:
    return {handle: _task_from_spec(spec) for handle, spec in specs.items()}


_CORPUS = _load_corpus()


@pytest.mark.parametrize("case", _CORPUS, ids=[c["name"] for c in _CORPUS])
def test_corpus_parity(case: Dict[str, object]) -> None:
    """Every golden case: success → lowering deep-equals ``expect``; error → the
    matching error class is raised."""
    tasks = _tasks_from_spec(case["tasks"])
    seed = case.get("seed", 0)
    context = case.get("context_bundles")  # PR-7b: chain-level attachment (None ⇒ [])

    if "error" in case:
        expected_exc = _ERROR_CLASSES[case["error"]]
        with pytest.raises(expected_exc):
            # The cycle check fires at lowering time; parse/handle at parse time.
            chain(case["dsl"], tasks, seed=seed, context=context).lowering()
        return

    lowered = chain(case["dsl"], tasks, seed=seed, context=context).lowering()
    # PR-7b: existing cases omit `context_bundles` in `expect` ⇒ default it to []
    # (matches the SPEC "absent ⇒ []" rule + Rust `#[serde(default)]`).
    expected = dict(case["expect"])
    expected.setdefault("context_bundles", [])
    assert lowered == expected


@pytest.mark.parametrize(
    "case",
    [c for c in _CORPUS if "expect" in c],
    ids=[c["name"] for c in _CORPUS if "expect" in c],
)
def test_corpus_export_import_round_trip(case: Dict[str, object]) -> None:
    """Batch B (D161.2): every corpus SUCCESS case round-trips through the portable
    blueprint artifact — ``to_blueprint()`` → JSON → ``from_blueprint()`` re-compiles
    to the IDENTICAL ``SubmitWorkflowRequest`` as ``build()`` (the cross-surface
    export/import guarantee), with no new corpus cases needed."""
    tasks = _tasks_from_spec(case["tasks"])
    seed = case.get("seed", 0)
    context = case.get("context_bundles")
    c = chain(case["dsl"], tasks, seed=seed, context=context)

    bp = c.to_blueprint()
    # The artifact must survive a JSON round-trip (a real file write/read).
    reparsed = json.loads(json.dumps(bp))
    assert Chain.from_blueprint(reparsed) == c.build(), case["name"]


def test_seed_flows_to_the_request() -> None:
    """``seed`` is opaque to the lowering inspector but reaches the built request."""
    tasks = {"a": pure(), "b": pure()}
    req = chain("a > b", tasks, seed=7).build()
    assert req.seed == 7
    assert len(req.steps) == 2
    assert len(req.edges) == 1


def test_build_produces_a_frozen_request() -> None:
    from kortecx.v1 import gateway_pb2 as g

    req = chain("a > b & c", {"a": pure(), "b": pure(), "c": pure()}).build()
    assert req.execution_mode == g.WorkflowExecutionMode.WORKFLOW_EXECUTION_MODE_FROZEN
    # a > b & c : only one edge, a->b (precedence: > tighter than &)
    assert len(req.edges) == 1
    assert (req.edges[0].parent, req.edges[0].child) == (0, 1)
    # PR-7b: a chain with no attached context carries an empty repeated field
    # (byte-identical to pre-PR-7).
    assert list(req.context_bundles) == []


# --- PR-7b: context bundles (chain-level attachment) --------------------------


def test_context_kwarg_flows_to_the_request() -> None:
    """``context=`` reaches ``SubmitWorkflowRequest.context_bundles`` verbatim."""
    req = chain("a > b", {"a": pure(), "b": pure()}, context=["team/ctx/spec"]).build()
    assert list(req.context_bundles) == ["team/ctx/spec"]
    assert len(req.steps) == 2  # context is chain-level, NOT a step


def test_context_is_emitted_in_the_lowering() -> None:
    lowered = chain("a", {"a": pure()}, context=["x/y/z"]).lowering()
    assert lowered["context_bundles"] == ["x/y/z"]
    assert len(lowered["steps"]) == 1


def test_context_order_is_preserved_not_sorted() -> None:
    """The DSL never sorts/dedups — the SERVER canonicalizes at bind (SN-8)."""
    handles = ["z/ctx/two", "a/ctx/one"]
    req = chain("a", {"a": pure()}, context=handles).build()
    assert list(req.context_bundles) == handles


def test_fluent_context_matches_kwarg_and_appends() -> None:
    base = chain("a > b", {"a": pure(), "b": pure()})
    via_fluent = base.context("team/ctx/spec").context("team/ctx/notes")
    via_kwarg = chain(
        "a > b", {"a": pure(), "b": pure()}, context=["team/ctx/spec", "team/ctx/notes"]
    )
    assert via_fluent.lowering() == via_kwarg.lowering()
    # `.context()` is immutable — the base chain is unchanged.
    assert base.lowering()["context_bundles"] == []


def test_sugar_context_matches_string() -> None:
    a, b = pure(), pure()
    sugar = Chain.from_node(a >> b, context=["team/ctx/spec"])
    string = chain("a > b", {"a": pure(), "b": pure()}, context=["team/ctx/spec"])
    assert sugar.lowering() == string.lowering()


# --- operator sugar lowers identically to the string DSL ----------------------


def _both_match(operator_chain: Chain, dsl: str, tasks: Dict[str, Task]) -> None:
    """Assert the operator AST and the string DSL lower to the same canonical
    form (modulo identical task payloads)."""
    assert operator_chain.lowering() == chain(dsl, tasks).lowering()


def test_sugar_seq2_matches_string() -> None:
    a, b = pure(), pure()
    _both_match(Chain.from_node(a >> b), "a > b", {"a": pure(), "b": pure()})


def test_sugar_fanout_group_matches_string() -> None:
    # a > [b & c]  ==  a >> (b & c)
    a, b, c = pure(), pure(), pure()
    _both_match(
        Chain.from_node(a >> (b & c)),
        "a > [b & c]",
        {"a": pure(), "b": pure(), "c": pure()},
    )


def test_sugar_fanin_group_matches_string() -> None:
    # [a & b] > c  ==  (a & b) >> c
    a, b, c = pure(), pure(), pure()
    _both_match(
        Chain.from_node((a & b) >> c),
        "[a & b] > c",
        {"a": pure(), "b": pure(), "c": pure()},
    )


def test_sugar_bipartite_matches_string() -> None:
    # [a & b] > [c & d]
    a, b, c, d = pure(), pure(), pure(), pure()
    _both_match(
        Chain.from_node((a & b) >> (c & d)),
        "[a & b] > [c & d]",
        {"a": pure(), "b": pure(), "c": pure(), "d": pure()},
    )


def test_sugar_precedence_amp_tighter_than_pipe() -> None:
    # a | b & c  ==  a | (b & c) — all parallel, no edges; & tighter than |
    a, b, c = pure(), pure(), pure()
    _both_match(
        Chain.from_node(a | b & c),
        "a | b & c",
        {"a": pure(), "b": pure(), "c": pure()},
    )


def test_sugar_object_identity_reuses_the_node() -> None:
    """Reusing the SAME Task object twice is the SAME node (the DAG reuse rule):
    ``a >> b | a >> c`` is a 3-node fan-out, matching ``a > b | a > c``."""
    a, b, c = pure(), pure(), pure()
    sugar = Chain.from_node((a >> b) | (a >> c))
    lowered = sugar.lowering()
    assert len(lowered["steps"]) == 3
    assert lowered["edges"] == [
        {"parent": 0, "child": 1, "edge": "data"},
        {"parent": 0, "child": 2, "edge": "data"},
    ]
    _both_match(sugar, "a > b | a > c", {"a": pure(), "b": pure(), "c": pure()})


def test_sugar_model_step_matches_string() -> None:
    gen = model("kx-serve:qwen3-4b-q4_k_m", "Summarize the input.")
    summarize = pure(label="final")
    sugar = Chain.from_node(gen >> summarize)
    dsl_tasks = {
        "gen": model("kx-serve:qwen3-4b-q4_k_m", "Summarize the input."),
        "sum": pure(label="final"),
    }
    _both_match(sugar, "gen > sum", dsl_tasks)


def test_sugar_self_loop_raises_cycle() -> None:
    a = pure()
    with pytest.raises(ChainCycleError):
        Chain.from_node(a >> a).lowering()


# --- PR-9b: the deterministic-agentic step (`@` grammar / `tools=`) ------------


def test_sugar_agentic_step_matches_at_grammar() -> None:
    """`model(tools=[...])` lowers IDENTICALLY to the string DSL `handle@tool`."""
    sugar = Chain.from_node(model("kx-serve:qwen3-4b-q4_k_m", "go", tools=["echo"]))
    string = chain("p@echo", {"p": model("kx-serve:qwen3-4b-q4_k_m", "go")})
    assert sugar.lowering() == string.lowering()
    step = sugar.lowering()["steps"][0]
    assert step["kind"] == "model"
    assert step["tool_contract"] == {"echo": "1"}


def test_agentic_budget_lowers_to_params() -> None:
    lowered = chain(
        "p@echo",
        {"p": model("kx-serve:qwen3-4b-q4_k_m", "go", max_turns=4, max_tool_calls=3)},
    ).lowering()
    assert lowered["steps"][0]["params"] == {"max_turns": "4", "max_tool_calls": "3"}


def test_agentic_grants_on_non_model_raise() -> None:
    with pytest.raises(AgenticStepError):
        chain("p@echo", {"p": pure()}).lowering()


# ---- Batch A: omittable model_id + reasoning= veneer ----


def test_model_id_is_optional_and_lowers_to_empty() -> None:
    # Omit model_id (prompt-only) — the server binds the served model (SN-8). The
    # lowering carries an empty model_id; a default_model fills it client-side at submit.
    lowered = chain("p > q", {"p": model(prompt="go"), "q": pure()}).lowering()
    assert lowered["steps"][0]["kind"] == "model"
    assert lowered["steps"][0]["model_id"] == ""
    # A named model still lowers verbatim (positional, unchanged).
    lowered2 = chain("p", {"p": model("kx-serve:m", "go")}).lowering()
    assert lowered2["steps"][0]["model_id"] == "kx-serve:m"


def test_reasoning_kwarg_lowers_to_params() -> None:
    for mode in ("full", "minimal", "off", "strip"):
        lowered = chain("p", {"p": model("kx-serve:m", "go", reasoning=mode)}).lowering()
        assert lowered["steps"][0]["params"] == {"reasoning": mode}
    # Absent ⇒ byte-identical (no reasoning key).
    assert chain("p", {"p": model("kx-serve:m", "go")}).lowering()["steps"][0]["params"] == {}


def test_reasoning_invalid_value_is_rejected() -> None:
    with pytest.raises(ChainError):
        model("kx-serve:m", "go", reasoning="loud")


def test_fill_default_model_fills_only_empty_model_steps() -> None:
    from kortecx.client import _fill_default_model

    req = chain(
        "p > q > r",
        {
            "p": model(prompt="go"),  # empty model_id ⇒ filled
            "q": model("kx-serve:explicit", "go"),  # named ⇒ untouched
            "r": pure(),  # pure ⇒ untouched
        },
    ).build()
    _fill_default_model(req, "kx-serve:default")
    assert req.steps[0].model_id == "kx-serve:default"
    assert req.steps[1].model_id == "kx-serve:explicit"
    assert req.steps[2].model_id == ""
    # No default ⇒ no-op (server binds "" → served).
    req2 = chain("p", {"p": model(prompt="go")}).build()
    _fill_default_model(req2, "")
    assert req2.steps[0].model_id == ""
