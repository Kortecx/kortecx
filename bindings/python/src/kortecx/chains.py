"""The Chains DSL — compose task handles into a Tier-1 DAG with operators or a
string expression, then lower to a ``SubmitWorkflowRequest`` via the existing
:class:`~kortecx.blueprints.BlueprintBuilder`.

This is the Python surface of the cross-surface chain contract pinned by
``tests/golden/chains/SPEC.md`` + ``corpus.json`` (the GR12 tri-surface parity
gate). The grammar, precedence, lowering, and error classes match the TypeScript
and Rust (CLI) implementations byte-for-byte.

SN-8: a chain describes TOPOLOGY only. It never computes a MoteId or a warrant —
it assembles the steps + edges the SERVER compiles + admits (the
:class:`~kortecx.blueprints.BlueprintBuilder` contract). A tampered chain only
changes what is PROPOSED, never what identity it gets.

Two front doors, one lowering:

- **operator sugar** — ``a >> b`` (sequential, a DATA edge), ``a & b`` / ``a | b``
  (parallel merge, no edge). Operators build an AST over :class:`Task` nodes;
  object identity IS node identity, so reusing the same :class:`Task` twice is the
  same node (the DAG reuse rule). ``&`` binds tighter than ``|`` and ``>>`` tighter
  than both, matching Python's native operator precedence and the string DSL.
- **string DSL** — :func:`chain` parses the exact grammar (recursive descent) and
  resolves handles via a caller ``tasks`` map.

Both produce a :class:`Chain`; :meth:`Chain.build` runs the one canonical lowering.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Dict, List, Optional, Tuple, Union

from .blueprints import TOOL_ARGS_KEY, BlueprintBuilder, EdgeInput, StepInput
from .v1 import gateway_pb2 as _g


class ChainError(ValueError):
    """Base for every chain authoring error (a :class:`ValueError` subclass)."""


class ChainParseError(ChainError):
    """The expression (or a group) is empty / malformed — error class ``parse``."""


class UnknownHandleError(ChainError):
    """A parsed handle is absent from the ``tasks`` map — class ``unknown_handle``."""


class ChainCycleError(ChainError):
    """The lowered topology has a cycle / self-loop — error class ``cycle``."""


# --- Task nodes + the operator AST --------------------------------------------


@dataclass(eq=False)
class Task:
    """One task node carrying a :class:`~kortecx.blueprints.StepInput` payload.

    ``eq=False`` keeps the default identity hash/eq: a :class:`Task` is its own
    node, so reusing one object twice in an expression reuses the node (the DAG
    reuse rule). Build one via :func:`pure` / :func:`model`.
    """

    step: StepInput

    # --- operator sugar (lower identically to the string DSL) ---
    def __rshift__(self, other: "_Node") -> "_Seq":
        """``a >> b`` — sequential (a DATA edge a→b)."""
        return _Seq([_as_node(self), _as_node(other)])

    def __and__(self, other: "_Node") -> "_Par":
        """``a & b`` — parallel merge (no edge)."""
        return _Par([_as_node(self), _as_node(other)])

    def __or__(self, other: "_Node") -> "_Par":
        """``a | b`` — parallel merge (no edge); same operation as ``&``."""
        return _Par([_as_node(self), _as_node(other)])


@dataclass
class _Seq:
    """A left-folded sequential fragment (``A > B > ...``)."""

    parts: List["_Node"]

    def __rshift__(self, other: "_Node") -> "_Seq":
        return _Seq([*self.parts, _as_node(other)])

    def __and__(self, other: "_Node") -> "_Par":
        return _Par([self, _as_node(other)])

    def __or__(self, other: "_Node") -> "_Par":
        return _Par([self, _as_node(other)])


@dataclass
class _Par:
    """A left-folded parallel-merge fragment (``A & B`` / ``A | B``)."""

    parts: List["_Node"]

    def __rshift__(self, other: "_Node") -> "_Seq":
        return _Seq([self, _as_node(other)])

    def __and__(self, other: "_Node") -> "_Par":
        return _Par([*self.parts, _as_node(other)])

    def __or__(self, other: "_Node") -> "_Par":
        return _Par([*self.parts, _as_node(other)])


# A node in the operator AST: a leaf Task or a composed fragment.
_Node = Union[Task, _Seq, _Par]


def _as_node(x: "_Node") -> "_Node":
    if isinstance(x, (Task, _Seq, _Par)):
        return x
    raise TypeError(f"not a chain node: {x!r}")


# --- Task factories -----------------------------------------------------------


def pure(**params: Union[bytes, str]) -> Task:
    """A PURE step (deterministic, no model/egress). ``params`` are step params
    (``str`` UTF-8-encoded at ``build()`` time, or ``bytes`` verbatim)."""
    return Task(StepInput(kind="pure", params=dict(params)))


def model(model_id: str, prompt: str, **params: Union[bytes, str]) -> Task:
    """A MODEL step. ``model_id`` is the recipe enum the SERVER validates (SN-8);
    ``prompt`` is the instruction; ``params`` are extra step params."""
    return Task(StepInput(kind="model", model_id=model_id, prompt=prompt, params=dict(params)))


def _canonical_args_json(args: Dict[str, object]) -> str:
    """Serialize a flat tool-call arg map to the canonical-JSON string the three
    SDK surfaces lower byte-identically (sorted keys, compact separators). No
    floats (SN-8 — the server schema is integer/bytes/bool/enum-typed)."""
    return json.dumps(args, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def tool(tool_id: str, tool_version: str, **args: object) -> Task:
    """A TOOL step (PR-6b-2): fire a single REGISTERED tool as a standalone node.

    ``tool_id`` + ``tool_version`` name the tool the SERVER resolves in its live
    registry (SN-8 — the client never supplies the warrant); ``args`` are the
    tool-call arguments, lowered to ONE canonical-JSON object under
    :data:`~kortecx.blueprints.TOOL_ARGS_KEY` in the step's params. The coordinator
    re-derives + validates those args against the tool's typed schema fail-closed.
    """
    return Task(
        StepInput(
            kind="tool",
            tool_contract={tool_id: tool_version},
            params={TOOL_ARGS_KEY: _canonical_args_json(dict(args))},
        )
    )


# --- The chain (operator AST or parsed DSL) + lowering ------------------------

# A fragment is a (entries, exits) pair over the shared ordered node list — the
# same shape the SPEC defines. Entries/exits are Task identities.
_Fragment = Tuple[List[Task], List[Task]]


class Chain:
    """A composed chain ready to lower. Build one from operator sugar via
    :meth:`from_node`, or from the string DSL via :func:`chain`."""

    def __init__(self, root: "_Node", *, seed: int = 0) -> None:
        self._root = root
        self._seed = seed

    @classmethod
    def from_node(cls, node: "_Node", *, seed: int = 0) -> "Chain":
        """Wrap an operator-sugar AST (``a >> b``, ``a & b``, ...) as a chain."""
        return cls(_as_node(node), seed=seed)

    # --- lowering -------------------------------------------------------------
    def _lower(self) -> Tuple[List[Task], List[Tuple[int, int]]]:
        """Walk the AST → (ordered node list, sorted-deduped edge list). The node
        list is in first-appearance order; edges are ``(parent_index, child_index)``
        deduped + sorted ascending. Raises :class:`ChainCycleError` on a cycle."""
        nodes: List[Task] = []
        index: Dict[int, int] = {}  # id(Task) -> node index
        edges: set[Tuple[int, int]] = set()

        def node_index(t: Task) -> int:
            key = id(t)
            i = index.get(key)
            if i is None:
                i = len(nodes)
                index[key] = i
                nodes.append(t)
            return i

        def visit(n: "_Node") -> _Fragment:
            if isinstance(n, Task):
                node_index(n)  # register on first appearance
                return ([n], [n])
            if isinstance(n, _Seq):
                # left-fold: edge every left exit -> every right entry
                entries, exits = visit(n.parts[0])
                for part in n.parts[1:]:
                    p_entries, p_exits = visit(part)
                    for x in exits:
                        for y in p_entries:
                            edges.add((node_index(x), node_index(y)))
                    exits = p_exits
                return (entries, exits)
            if isinstance(n, _Par):
                # left-fold: order-preserving dedup of entries/exits, no edges
                entries = []
                exits = []
                for part in n.parts:
                    p_entries, p_exits = visit(part)
                    for e in p_entries:
                        if e not in entries:
                            entries.append(e)
                    for e in p_exits:
                        if e not in exits:
                            exits.append(e)
                return (entries, exits)
            raise TypeError(f"not a chain node: {n!r}")

        visit(self._root)
        sorted_edges = sorted(edges)
        _check_acyclic(len(nodes), sorted_edges)
        return nodes, sorted_edges

    def lowering(self) -> Dict[str, object]:
        """The canonical pre-encoding lowering (a pure dict, params as STRINGS) for
        the corpus parity test. Shape matches ``corpus.json``'s ``expect``:
        ``{steps:[{kind,model_id,prompt,body_signature_id,tool_contract,params}],
        edges:[{parent,child,edge}]}``."""
        nodes, edges = self._lower()
        steps = [
            {
                "kind": t.step.kind,
                "model_id": t.step.model_id,
                "prompt": t.step.prompt,
                "body_signature_id": t.step.body_signature_id,
                "tool_contract": dict(t.step.tool_contract),
                "params": {k: _as_str(v) for k, v in t.step.params.items()},
            }
            for t in nodes
        ]
        edge_rows = [{"parent": p, "child": c, "edge": "data"} for (p, c) in edges]
        return {"steps": steps, "edges": edge_rows}

    def build(self) -> "_g.SubmitWorkflowRequest":
        """Lower → :class:`~kortecx.blueprints.BlueprintBuilder` → the request. Nodes
        feed ``add_step`` in first-appearance order; the sorted deduped data edges
        feed ``add_edge``; ``seed`` is the chain's seed; mode is ``frozen``."""
        nodes, edges = self._lower()
        builder = BlueprintBuilder(self._seed)
        for t in nodes:
            builder.add_step(t.step)
        for parent, child in edges:
            builder.add_edge(EdgeInput(parent=parent, child=child, edge="data"))
        return builder.build()


def _as_str(v: Union[bytes, str]) -> str:
    """Render a param value as the pre-encoding string the lowering compares."""
    return v if isinstance(v, str) else v.decode("utf-8")


def _check_acyclic(node_count: int, edges: List[Tuple[int, int]]) -> None:
    """Kahn topo check; raise :class:`ChainCycleError` if any node never drains
    (the DSL can express cycles + self-loops via handle reuse)."""
    indegree = [0] * node_count
    adj: List[List[int]] = [[] for _ in range(node_count)]
    for parent, child in edges:
        adj[parent].append(child)
        indegree[child] += 1
    queue = [i for i in range(node_count) if indegree[i] == 0]
    visited = 0
    while queue:
        n = queue.pop()
        visited += 1
        for child in adj[n]:
            indegree[child] -= 1
            if indegree[child] == 0:
                queue.append(child)
    if visited != node_count:
        raise ChainCycleError("chain has a cycle")


# --- The string DSL — recursive-descent parser --------------------------------


class _Parser:
    """Recursive-descent parser for the chain grammar (see SPEC.md). Resolves each
    handle via ``tasks`` and builds the operator AST (so the DSL and operator sugar
    share the single lowering)."""

    def __init__(self, expr: str, tasks: Dict[str, Task]) -> None:
        self._src = expr
        self._tasks = tasks
        self._toks = _tokenize(expr)
        self._pos = 0

    def _peek(self) -> Optional[str]:
        return self._toks[self._pos] if self._pos < len(self._toks) else None

    def _next(self) -> str:
        tok = self._toks[self._pos]
        self._pos += 1
        return tok

    def parse(self) -> "_Node":
        node = self._or_expr()
        if self._peek() is not None:
            raise ChainParseError(f"unexpected token {self._peek()!r}")
        return node

    def _or_expr(self) -> "_Node":
        node = self._and_expr()
        while self._peek() == "|":
            self._next()
            right = self._and_expr()
            node = node | right
        return node

    def _and_expr(self) -> "_Node":
        node = self._seq_expr()
        while self._peek() == "&":
            self._next()
            right = self._seq_expr()
            node = node & right
        return node

    def _seq_expr(self) -> "_Node":
        node = self._atom()
        while self._peek() == ">":
            self._next()
            right = self._atom()
            node = node >> right
        return node

    def _atom(self) -> "_Node":
        tok = self._peek()
        if tok is None:
            raise ChainParseError("unexpected end of expression")
        if tok == "[":
            self._next()
            if self._peek() == "]":
                raise ChainParseError("empty group '[]'")
            inner = self._or_expr()
            if self._peek() != "]":
                raise ChainParseError("expected ']'")
            self._next()
            return inner
        if tok in ("|", "&", ">", "]"):
            raise ChainParseError(f"unexpected token {tok!r}")
        # a handle
        handle = self._next()
        task = self._tasks.get(handle)
        if task is None:
            raise UnknownHandleError(f"unknown task handle '{handle}'")
        return task


_HANDLE_START = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_"
_HANDLE_REST = _HANDLE_START + "0123456789-"


def _tokenize(expr: str) -> List[str]:
    """Split an expression into tokens (handles + the four operator/bracket
    chars). Whitespace is insignificant; any other char is a parse error."""
    toks: List[str] = []
    i = 0
    n = len(expr)
    while i < n:
        ch = expr[i]
        if ch.isspace():
            i += 1
            continue
        if ch in "|&>[]":
            toks.append(ch)
            i += 1
            continue
        if ch in _HANDLE_START:
            j = i + 1
            while j < n and expr[j] in _HANDLE_REST:
                j += 1
            toks.append(expr[i:j])
            i = j
            continue
        raise ChainParseError(f"unexpected character {ch!r}")
    return toks


def chain(expr: str, tasks: Dict[str, Task], *, seed: int = 0) -> Chain:
    """Parse a chain string expression (the exact grammar in SPEC.md), resolving
    handles via ``tasks``, into a :class:`Chain`.

    Raises :class:`ChainParseError` on an empty/malformed expression or empty
    group, :class:`UnknownHandleError` on a handle absent from ``tasks``, and
    (at lowering time) :class:`ChainCycleError` on a cycle / self-loop. Tasks
    defined but unused are ignored.
    """
    if not expr.strip():
        raise ChainParseError("empty chain expression")
    root = _Parser(expr, tasks).parse()
    return Chain(root, seed=seed)
