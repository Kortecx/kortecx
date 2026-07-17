"""Property / fuzz tests for the Chains string-DSL parser + lowering.

The Chains DSL accepts user-supplied text, so it gets round-trip + robustness
invariants asserted across the input space — not just the hand-picked cases in
``test_chains.py`` / the golden corpus. Covered:

* **precedence & edge generation** — ``>`` (seq, tightest) binds tighter than ``&``
  binds tighter than ``|``; over a flat chain of DISTINCT handles the DATA-edge set is
  exactly the adjacent ``>`` pairs (an INDEPENDENT oracle, not the parser's own output).
* **``@``-grants** — order-preserving dedup into a MODEL step's tool contract (each at
  version ``"1"``); a ``@``-grant on a non-model handle is a fail-closed authoring error.
* **handle dedup** — object/handle identity: reusing a handle is ONE node.
* **cycle detection** — any handle-reuse that closes a ``>`` loop raises ``ChainCycleError``.
* **fuzz robustness** — arbitrary text lowers or raises a ``ChainError`` subclass, never
  an unexpected ``KeyError`` / ``IndexError`` / ``RecursionError`` / etc.
* **blueprint round-trip** — ``from_blueprint(to_blueprint())`` re-compiles to the
  byte-identical ``SubmitWorkflowRequest`` as ``build()`` (deterministic serialization).

These need no server (the DSL describes topology; the SERVER compiles + warrants) so
they run offline and deterministically.
"""

from __future__ import annotations

from typing import Dict, List

from hypothesis import given, settings
from hypothesis import strategies as st

from kortecx.chains import (
    AgenticStepError,
    Chain,
    ChainCycleError,
    ChainError,
    Task,
    chain,
    model,
    pure,
)

_OPERATORS = [">", "&", "|"]


def _distinct_handles(n: int) -> List[str]:
    return [f"h{i}" for i in range(n)]


def _flat_expr(handles: List[str], ops: List[str]) -> str:
    """`h0 OP0 h1 OP1 h2 …` — a flat binary chain of distinct handles."""
    expr = handles[0]
    for handle, op in zip(handles[1:], ops):
        expr += f" {op} {handle}"
    return expr


# --- precedence + edge generation (independent oracle) -----------------------


@settings(max_examples=250)
@given(ops=st.lists(st.sampled_from(_OPERATORS), min_size=1, max_size=7))
def test_precedence_edges_are_the_adjacent_seq_pairs(ops: List[str]) -> None:
    """Over a flat chain of DISTINCT handles, the lowered DATA edges are EXACTLY the
    adjacent ``>`` pairs — regardless of the surrounding ``&`` / ``|`` — because ``>``
    binds tightest + left-assoc and ``&`` / ``|`` are edge-free parallel merges. This
    pins precedence, associativity, and edge derivation against an oracle the parser
    never computes."""
    n = len(ops) + 1
    handles = _distinct_handles(n)
    tasks: Dict[str, Task] = {h: pure() for h in handles}
    low = chain(_flat_expr(handles, ops), tasks).lowering()

    # steps are the handles in first-appearance order (all distinct here).
    assert [s["kind"] for s in low["steps"]] == ["pure"] * n
    expected = sorted((i, i + 1) for i, op in enumerate(ops) if op == ">")
    got = sorted((e["parent"], e["child"]) for e in low["edges"])
    assert got == expected
    # edges are canonically sorted + deduped.
    assert got == [(e["parent"], e["child"]) for e in low["edges"]]


@settings(max_examples=200)
@given(
    left=st.sampled_from(_OPERATORS),
    right=st.sampled_from(_OPERATORS),
)
def test_bracket_grouping_matches_precedence(left: str, right: str) -> None:
    """``a L b R c`` lowers identically to its precedence-parenthesized form — the
    tighter operator groups first. (`>` > `&` > `|`; equal ops are left-assoc.)"""
    order = {">": 0, "&": 1, "|": 2}
    tasks: Dict[str, Task] = {"a": pure(), "b": pure(), "c": pure()}
    flat = chain(f"a {left} b {right} c", tasks).lowering()
    if order[left] <= order[right]:
        grouped = chain(f"[a {left} b] {right} c", tasks).lowering()
    else:
        grouped = chain(f"a {left} [b {right} c]", tasks).lowering()
    assert flat == grouped


# --- @-grants: order-preserving dedup + non-model rejection ------------------


@settings(max_examples=200)
@given(tags=st.lists(st.sampled_from(["t0", "t1", "t2", "t3"]), min_size=0, max_size=8))
def test_grant_tags_dedup_order_preserving(tags: List[str]) -> None:
    """`m@t@t@…` folds the ``@``-tags into the MODEL step's tool_contract as an
    order-preserving dedup, each at version ``"1"``."""
    expr = "m" + "".join(f"@{t}" for t in tags)
    low = chain(expr, {"m": model(prompt="go")}).lowering()
    contract = low["steps"][0]["tool_contract"]
    # order-preserving dedup == dict.fromkeys.
    assert list(contract.keys()) == list(dict.fromkeys(tags))
    assert all(v == "1" for v in contract.values())


@settings(max_examples=100)
@given(tag=st.sampled_from(["t0", "t1", "t2"]))
def test_grant_on_non_model_is_rejected(tag: str) -> None:
    """A ``@``-grant on a PURE handle is a fail-closed authoring error (the
    deterministic-agentic step requires a MODEL step)."""
    try:
        chain(f"p@{tag}", {"p": pure()}).lowering()
        raise AssertionError("expected AgenticStepError")
    except AgenticStepError:
        pass


# --- handle dedup: reuse is one node ----------------------------------------


@settings(max_examples=150)
@given(count=st.integers(min_value=1, max_value=8))
def test_parallel_handle_reuse_is_one_node(count: int) -> None:
    """`a & a & … & a` (N reuses) lowers to exactly ONE node and ZERO edges — object/
    handle identity means a reused handle is the same node."""
    expr = " & ".join(["a"] * count)
    low = chain(expr, {"a": pure()}).lowering()
    assert len(low["steps"]) == 1
    assert low["edges"] == []


@settings(max_examples=150)
@given(
    n=st.integers(min_value=2, max_value=6),
    reuse=st.integers(min_value=0, max_value=5),
)
def test_node_count_equals_distinct_handles(n: int, reuse: int) -> None:
    """A parallel merge of a multiset of handles has exactly ``len(set(handles))``
    nodes (dedup) and no edges."""
    handles = _distinct_handles(n)
    multiset = handles + handles[: reuse % (n + 1)]
    tasks: Dict[str, Task] = {h: pure() for h in handles}
    low = chain(" & ".join(multiset), tasks).lowering()
    assert len(low["steps"]) == len(set(multiset))
    assert low["edges"] == []


# --- cycle detection ---------------------------------------------------------


@settings(max_examples=150)
@given(n=st.integers(min_value=1, max_value=6))
def test_closing_a_seq_loop_raises_cycle(n: int) -> None:
    """`h0 > h1 > … > h(n-1) > h0` reuses ``h0`` to close a ``>`` loop — always a
    cycle. (n == 1 is the ``a > a`` self-loop.)"""
    handles = _distinct_handles(n)
    tasks: Dict[str, Task] = {h: model(prompt="x") for h in handles}
    expr = " > ".join([*handles, handles[0]])
    try:
        chain(expr, tasks).lowering()
        raise AssertionError("expected ChainCycleError")
    except ChainCycleError:
        pass


# --- fuzz robustness ---------------------------------------------------------

# A small alphabet biased toward real DSL bytes (handles, operators, brackets, `@`,
# whitespace) plus a little junk — so the fuzzer hits both parse errors and the
# success path.
_FUZZ_ALPHABET = "abcdАh0123_-@&|[]> \t.$"


@settings(max_examples=600)
@given(text=st.text(alphabet=_FUZZ_ALPHABET, max_size=48))
def test_arbitrary_text_only_raises_chain_error(text: str) -> None:
    """The parser + lowering over ARBITRARY text either succeeds or raises a
    ``ChainError`` subclass — never an unexpected exception (``KeyError`` /
    ``IndexError`` / ``RecursionError`` / ``TypeError`` / …)."""
    tasks: Dict[str, Task] = {"a": pure(), "b": model(prompt="x"), "c": pure()}
    try:
        chain(text, tasks).lowering()
    except ChainError:
        pass  # every declared authoring error is a ChainError (a ValueError subclass)


# --- portable-blueprint round-trip ------------------------------------------


@settings(max_examples=150)
@given(ops=st.lists(st.sampled_from(_OPERATORS), min_size=0, max_size=5))
def test_blueprint_round_trips_to_identical_request(ops: List[str]) -> None:
    """`Chain.from_blueprint(c.to_blueprint())` re-compiles to the byte-identical
    ``SubmitWorkflowRequest`` as ``c.build()`` (deterministic serialization) — the
    export/import artifact is a faithful round-trip."""
    n = len(ops) + 1
    handles = _distinct_handles(n)
    # alternate model/pure so the artifact carries both kinds + prompts.
    tasks: Dict[str, Task] = {
        h: (model(prompt=f"p{i}") if i % 2 == 0 else pure()) for i, h in enumerate(handles)
    }
    c = chain(_flat_expr(handles, ops), tasks)
    built = c.build().SerializeToString(deterministic=True)
    round_tripped = Chain.from_blueprint(c.to_blueprint()).SerializeToString(deterministic=True)
    assert built == round_tripped
