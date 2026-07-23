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
import os
from dataclasses import dataclass, field, replace
from typing import Dict, List, Mapping, Optional, Sequence, Tuple, Union

from .blueprints import (
    REACT_MAX_TOOL_CALLS_KEY,
    REACT_MAX_TURNS_KEY,
    REASONING_KEY,
    TOOL_ARGS_KEY,
    BlueprintBuilder,
    EdgeInput,
    StepInput,
)
from .v1 import gateway_pb2 as _g


class ChainError(ValueError):
    """Base for every chain authoring error (a :class:`ValueError` subclass)."""


class ChainParseError(ChainError):
    """The expression (or a group) is empty / malformed — error class ``parse``."""


class UnknownHandleError(ChainError):
    """A parsed handle is absent from the ``tasks`` map — class ``unknown_handle``."""


class ChainCycleError(ChainError):
    """The lowered topology has a cycle / self-loop — error class ``cycle``."""


class AgenticStepError(ChainError):
    """`@` tool grants were tagged onto a non-model handle — class
    ``agentic_non_model`` (the deterministic-agentic step requires a MODEL step)."""


# --- Task nodes + the operator AST --------------------------------------------


@dataclass(eq=False)
class Task:
    """One task node carrying a :class:`~kortecx.blueprints.StepInput` payload.

    ``eq=False`` keeps the default identity hash/eq: a :class:`Task` is its own
    node, so reusing one object twice in an expression reuses the node (the DAG
    reuse rule). Build one via :func:`pure` / :func:`model`.

    ``app_skills`` / ``app_connections`` / ``app_datasets`` are the per-node capability
    BINDINGS — catalog names this step uses when the chain becomes an App (:func:`app`).
    They name entries in the App envelope's ``references``, so they are meaningful ONLY on
    the App path; the plain workflow lowering (:meth:`Chain.build` /
    :meth:`Chain.from_blueprint`) has no ``references`` to resolve them against and REFUSES a
    non-empty one. Off the wire: they ride in the blueprint JSON, never in a StepInput.
    """

    step: StepInput
    app_skills: List[str] = field(default_factory=list)
    app_connections: List[str] = field(default_factory=list)
    app_datasets: List[str] = field(default_factory=list)

    def has_app_bindings(self) -> bool:
        """True when this step carries an App-envelope capability binding."""
        return bool(self.app_skills or self.app_connections or self.app_datasets)

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


def _grants_to_contract(
    tools: "Optional[Union[Sequence[str], Mapping[str, str]]]",
) -> Dict[str, str]:
    """Normalize an agentic-step tool grant set to a ``{name: version}`` contract: a
    sequence of names → version ``"1"`` (mirrors the ``@tool`` grammar default), a
    mapping → verbatim ``{name: version}``. Order-preserving dedup."""
    if tools is None:
        return {}
    if isinstance(tools, Mapping):
        return {str(k): str(v) for k, v in tools.items()}
    contract: Dict[str, str] = {}
    for name in tools:
        contract.setdefault(str(name), "1")
    return contract


#: Batch A: the opt-in reasoning modes a ``reasoning=`` kwarg accepts (the SERVER reads
#: ``config_subset["reasoning"]``). Any other value is a client-side error (fail-closed
#: at authoring rather than a silent server no-op).
_REASONING_MODES = frozenset({"full", "minimal", "off", "strip"})


def _validate_reasoning(reasoning: str) -> str:
    if reasoning not in _REASONING_MODES:
        raise ChainError(f"reasoning must be one of {sorted(_REASONING_MODES)}, got {reasoning!r}")
    return reasoning


def model(
    model_id: str = "",
    prompt: str = "",
    *,
    tools: "Optional[Union[Sequence[object], Mapping[str, str]]]" = None,
    max_turns: Optional[int] = None,
    max_tool_calls: Optional[int] = None,
    reasoning: Optional[str] = None,
    skills: "Optional[Sequence[str]]" = None,
    connections: "Optional[Sequence[str]]" = None,
    datasets: "Optional[Sequence[str]]" = None,
    **params: Union[bytes, str],
) -> Task:
    """A MODEL step. ``prompt`` is the instruction; ``params`` are extra step params.

    Batch A: ``model_id`` is OPTIONAL — omit it (or pass ``""``) and the SERVER binds
    the served model (SN-8); set a client ``default_model`` to fill it client-side, or
    name a specific served model. ``reasoning`` (``"full"`` / ``"minimal"`` / ``"off"``
    / ``"strip"``) sets the opt-in reasoning mode — absent ⇒ the model's own behavior
    (and a byte-identical MoteId). Use ``reasoning=`` as the typed knob rather than a
    raw ``params`` magic-string.

    PR-9b (D161.1): pass ``tools`` (a list of names → version ``"1"``, or a
    ``{name: version}`` map) to make this a **deterministic-agentic step** — the
    model runs a bounded reason→tool→observe loop over the granted tool SET (the
    same step the string DSL authors as ``handle@tool@tool``). ``max_turns`` /
    ``max_tool_calls`` bound the loop (default 8 / 20 — decoupled, a turn can fire
    N tools; ignored when no tools).

    V2b: ``tools`` may also include ``@kx.tool``-decorated LOCAL functions — the SDK
    registers each as a stdio MCP server at the run terminal and fills the
    server-derived name into the contract (off the lowering until then)."""
    from .tools import split_tools

    str_tools, local_tools = split_tools(tools)
    step_params: Dict[str, Union[bytes, str]] = dict(params)
    if reasoning is not None:
        step_params[REASONING_KEY] = _validate_reasoning(reasoning)
    return Task(
        StepInput(
            kind="model",
            model_id=model_id,
            prompt=prompt,
            tool_contract=_grants_to_contract(str_tools),
            params=step_params,
            max_turns=max_turns,
            max_tool_calls=max_tool_calls,
            local_tools=local_tools,
        ),
        app_skills=list(skills or []),
        app_connections=list(connections or []),
        app_datasets=list(datasets or []),
    )


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

    def __init__(
        self,
        root: "_Node",
        *,
        seed: int = 0,
        context_bundles: Optional[Sequence[str]] = None,
    ) -> None:
        self._root = root
        self._seed = seed
        # PR-7b: chain-level context-bundle handles (verbatim caller order — the
        # SERVER canonicalizes the sorted ref-set into each entry Mote at bind, SN-8).
        self._context: List[str] = list(context_bundles or [])

    @classmethod
    def from_node(
        cls,
        node: "_Node",
        *,
        seed: int = 0,
        context: Optional[Sequence[str]] = None,
    ) -> "Chain":
        """Wrap an operator-sugar AST (``a >> b``, ``a & b``, ...) as a chain.

        ``context`` is an optional list of context-bundle handles to attach to the
        run (PR-7b) — chain-level, not a node (see :meth:`context`)."""
        return cls(_as_node(node), seed=seed, context_bundles=context)

    def context(self, *handles: str) -> "Chain":
        """Attach context-bundle ``handles`` to this chain (PR-7b), returning a NEW
        :class:`Chain` (immutable — the existing one is unchanged). Repeated calls
        APPEND in order; the SERVER resolves each handle to its content-refs and
        folds the sorted set into every entry Mote's identity-bearing config, so a
        different attached context ⇒ a different run (exactly-once-per-input+context).

        Context is request-level: it attaches to the chain's ENTRY Motes regardless
        of where this is called — there is no ``context`` step."""
        return Chain(
            self._root,
            seed=self._seed,
            context_bundles=[*self._context, *handles],
        )

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
                "params": _effective_params(t.step),
            }
            for t in nodes
        ]
        edge_rows = [{"parent": p, "child": c, "edge": "data"} for (p, c) in edges]
        # PR-7b: the chain-level context attachment, emitted verbatim (the corpus
        # pins its byte-identity across surfaces; absent ⇒ []).
        return {"steps": steps, "edges": edge_rows, "context_bundles": list(self._context)}

    def _iter_steps(self) -> List[StepInput]:
        """The lowered steps (in first-appearance order) — the live ``StepInput``
        objects, so the V2b local-tool resolver can fill resolved names into their
        ``tool_contract`` in place before :meth:`build` reads them."""
        nodes, _ = self._lower()
        return [t.step for t in nodes]

    def build(self) -> "_g.SubmitWorkflowRequest":
        """Lower → :class:`~kortecx.blueprints.BlueprintBuilder` → the request. Nodes
        feed ``add_step`` in first-appearance order; the sorted deduped data edges
        feed ``add_edge``; ``seed`` is the chain's seed; mode is ``frozen``; the
        chain's context bundles (if any) ride on the request (PR-7b)."""
        nodes, edges = self._lower()
        builder = BlueprintBuilder(self._seed)
        for i, t in enumerate(nodes):
            _refuse_app_bindings_on_workflow(i, t)
            builder.add_step(replace(t.step, params=_effective_params(t.step)))
        for parent, child in edges:
            builder.add_edge(EdgeInput(parent=parent, child=child, edge="data"))
        builder.context_bundles(self._context)
        return builder.build()

    # --- Batch B (D161.2): portable blueprint export / import -----------------
    def to_blueprint(self) -> Dict[str, object]:
        """Export this chain as a PORTABLE blueprint dict — the same shape
        ``kx blueprint run --file`` and :meth:`from_blueprint` consume. Round-trips:
        feeding it back to :meth:`from_blueprint` (or the CLI) re-compiles to the
        IDENTICAL :class:`SubmitWorkflowRequest` as :meth:`build`.

        ``params`` are in their FOLDED form (a tool step's args under
        ``kx.tool.args``; an agentic MODEL step's budget under ``max_turns`` /
        ``max_tool_calls``) — :meth:`from_blueprint` / the server import them WITHOUT
        re-folding (fold-idempotent). ``model_id`` is left as authored (empty ⇒ the
        server binds the served model, SN-8 — so the artifact is portable across
        serves). Empty fields are omitted for a clean artifact; each ``kind`` is
        explicit (self-describing)."""
        nodes, edges = self._lower()
        steps: List[Dict[str, object]] = []
        for t in nodes:
            s = t.step
            step: Dict[str, object] = {"kind": s.kind}
            if s.model_id:
                step["model_id"] = s.model_id
            if s.prompt:
                step["prompt"] = s.prompt
            if s.body_signature_id is not None:
                step["body_signature_id"] = s.body_signature_id
            if s.tool_contract:
                step["tool_contract"] = dict(s.tool_contract)
            params = _effective_params(s)
            if params:
                # params values are pre-encoding strings (the lowering form).
                step["params"] = {k: _as_str(v) for k, v in params.items()}
            # The per-node App capability bindings. Emitted only when non-empty, so a chain
            # that binds nothing produces byte-identical blueprint JSON.
            if t.app_skills:
                step["skills"] = list(t.app_skills)
            if t.app_connections:
                step["connections"] = list(t.app_connections)
            if t.app_datasets:
                step["datasets"] = list(t.app_datasets)
            steps.append(step)
        bp: Dict[str, object] = {
            "seed": self._seed,
            "execution_mode": "frozen",
            "steps": steps,
        }
        if edges:
            bp["edges"] = [{"parent": p, "child": c, "edge": "data"} for (p, c) in edges]
        if self._context:
            bp["context_bundles"] = list(self._context)
        return bp

    def export(self, path: Union[str, "os.PathLike[str]"]) -> None:
        """Write :meth:`to_blueprint` as pretty JSON to ``path`` — the portable
        artifact (save / version / share; re-run with ``kx blueprint run --file`` or
        :meth:`from_blueprint`)."""
        with open(path, "w", encoding="utf-8") as fh:
            json.dump(self.to_blueprint(), fh, indent=2)
            fh.write("\n")

    @classmethod
    def from_blueprint(cls, spec: Mapping[str, object]) -> "_g.SubmitWorkflowRequest":
        """Compile a portable blueprint dict (from :meth:`to_blueprint`, the CLI
        ``--emit-blueprint``, or a hand-authored DAG) into a
        :class:`SubmitWorkflowRequest` ready for ``client.submit_workflow``.

        Accepts BOTH artifact forms: the SDK FOLDED form (args/budget already in
        ``params``) and the CLI ARGS-SEPARATED form (a tool step's ``args`` map + an
        agentic step's ``max_turns`` / ``max_tool_calls`` as separate fields) — both
        fold to the same request. The chain TOPOLOGY is not recoverable from a DAG
        (only the request is), so this returns the request, not a :class:`Chain`."""
        builder = BlueprintBuilder(_opt_int(spec.get("seed")) or 0)
        raw_steps = _get(spec, "steps", [])
        if not isinstance(raw_steps, list):
            raise ChainError("blueprint `steps` must be a list")
        for i, d in enumerate(raw_steps):
            if not isinstance(d, Mapping):
                raise ChainError("each blueprint step must be an object")
            _refuse_spec_app_bindings(i, d)
            step = _step_from_spec(d)
            # Fold the budget exactly as `build()` does (a no-op when already folded).
            builder.add_step(replace(step, params=_effective_params(step)))
        raw_edges = _get(spec, "edges", [])
        for e in raw_edges if isinstance(raw_edges, list) else []:
            if not isinstance(e, Mapping):
                raise ChainError("each blueprint edge must be an object")
            parent = _opt_int(e.get("parent"))
            child = _opt_int(e.get("child"))
            if parent is None or child is None:
                raise ChainError("a blueprint edge needs integer `parent` + `child`")
            builder.add_edge(
                EdgeInput(
                    parent=parent,
                    child=child,
                    edge=str(e.get("edge", "data")),
                    non_cascade=bool(e.get("non_cascade", False)),
                )
            )
        ctx = _get(spec, "context_bundles", [])
        if isinstance(ctx, list):
            builder.context_bundles([str(h) for h in ctx])
        if _get(spec, "execution_mode", "frozen") == "dynamic":
            builder.mode("dynamic")
        return builder.build()

    @classmethod
    def from_blueprint_file(
        cls, path: "Union[str, os.PathLike[str]]"
    ) -> "_g.SubmitWorkflowRequest":
        """Read a portable blueprint JSON file and compile it (see
        :meth:`from_blueprint`)."""
        with open(path, encoding="utf-8") as fh:
            spec = json.load(fh)
        if not isinstance(spec, Mapping):
            raise ChainError("a blueprint file must be a JSON object")
        return cls.from_blueprint(spec)


def _get(m: "Mapping[str, object]", key: str, default: object) -> object:
    """Mapping ``.get`` with a default (a tiny helper so `from_blueprint` reads
    cleanly over an untyped JSON dict)."""
    return m.get(key, default)


def _infer_kind(d: "Mapping[str, object]") -> str:
    """Infer a step's kind from field presence when ``kind`` is omitted — mirrors the
    CLI ``StepSpec::resolve_kind`` (model fields win; then a tool contract; else pure).
    Our own exports always set ``kind`` explicitly; this covers hand-authored DAGs."""
    if d.get("model_id") or d.get("prompt"):
        return "model"
    if d.get("tool_contract"):
        return "tool"
    return "pure"


def _opt_int(v: object) -> Optional[int]:
    """An optional integer field from an untyped JSON value (``None`` stays ``None``)."""
    return int(v) if isinstance(v, (int, str)) else None


def _str_map(v: object) -> Dict[str, str]:
    """A ``{str: str}`` map from an untyped JSON value (non-maps ⇒ empty)."""
    return {str(k): str(val) for k, val in v.items()} if isinstance(v, Mapping) else {}


def _refuse_app_bindings_on_workflow(index: int, t: Task) -> None:
    """Refuse an App-envelope capability binding on the WORKFLOW lowering path.

    A ``SubmitWorkflow`` has no ``references`` for a ``skills`` / ``connections`` /
    ``datasets`` NAME to point at, so the runtime could only drop it. Fail at authoring with a
    message that says where the field IS honoured — mirroring the Rust
    ``kx_blueprint::to_request`` refusal, so all three surfaces agree.
    """
    if not t.has_app_bindings():
        return
    named = [
        name
        for name, present in (
            ("skills", bool(t.app_skills)),
            ("connections", bool(t.app_connections)),
            ("datasets", bool(t.app_datasets)),
        )
        if present
    ]
    raise ChainError(
        f"step {index} declares {' + '.join(named)} — a per-step capability list is an "
        "App-envelope binding that names an entry in the App's references, and RunApp is what "
        "resolves it. A workflow has no references to name into: author this as an App "
        "(app(...)), or grant the step a tool directly with tools=[...]."
    )


def _refuse_spec_app_bindings(index: int, d: "Mapping[str, object]") -> None:
    """As :func:`_refuse_app_bindings_on_workflow`, over a parsed blueprint step."""
    named = [
        name
        for name in ("skills", "connections", "datasets")
        if isinstance(d.get(name), list) and d.get(name)
    ]
    if not named:
        return
    raise ChainError(
        f"step {index} declares {' + '.join(named)} — a per-step capability list is an "
        "App-envelope binding resolved by RunApp; a workflow blueprint has no references to "
        "name into. Author it as an App, or grant the step a tool directly via tool_contract."
    )


def _step_from_spec(d: "Mapping[str, object]") -> StepInput:
    """One blueprint step dict → a :class:`StepInput`, folding the CLI args-separated
    form (a ``args`` map ⇒ ``kx.tool.args``) so both artifact forms import identically.
    The agentic budget rides as ``max_turns`` / ``max_tool_calls`` on the StepInput and
    is folded by `_effective_params` at build time (matching `Chain.build`)."""
    kind = str(d.get("kind") or _infer_kind(d))
    params: Dict[str, Union[bytes, str]] = {k: v for k, v in _str_map(d.get("params")).items()}
    args = d.get("args")
    if isinstance(args, Mapping) and args:
        params[TOOL_ARGS_KEY] = _canonical_args_json({str(k): v for k, v in args.items()})
    body_sig = d.get("body_signature_id")
    return StepInput(
        kind=kind,
        model_id=str(d.get("model_id", "")),
        prompt=str(d.get("prompt", "")),
        body_signature_id=str(body_sig) if body_sig is not None else None,
        tool_contract=_str_map(d.get("tool_contract")),
        params=params,
        max_turns=_opt_int(d.get("max_turns")),
        max_tool_calls=_opt_int(d.get("max_tool_calls")),
    )


def _as_str(v: Union[bytes, str]) -> str:
    """Render a param value as the pre-encoding string the lowering compares."""
    return v if isinstance(v, str) else v.decode("utf-8")


def _effective_params(step: StepInput) -> Dict[str, Union[bytes, str]]:
    """The step's params (as the pre-encoding string form the lowering compares),
    with the agentic-loop budget injected for a MODEL step carrying a non-empty
    ``tool_contract`` (PR-9b — mirrors the Rust ``to_request`` + the coordinator's
    canonical-JSON-``u32`` budget keys). Pure — never mutates the step. Absent budget
    ⇒ the coordinator default."""
    params: Dict[str, Union[bytes, str]] = {k: _as_str(v) for k, v in step.params.items()}
    if step.kind == "model" and step.tool_contract:
        if step.max_turns is not None:
            params[REACT_MAX_TURNS_KEY] = str(step.max_turns)
        if step.max_tool_calls is not None:
            params[REACT_MAX_TOOL_CALLS_KEY] = str(step.max_tool_calls)
    return params


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
        # PR-9b: handle → the (copied, possibly grant-augmented) node used in the
        # AST, so a reused handle is ONE node and `@`-grants accumulate on it
        # without mutating the caller's `tasks` map.
        self._nodes: Dict[str, Task] = {}

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
        # `@` at an atom start = a grant with no preceding handle (parse error).
        if tok in ("|", "&", ">", "]", "@"):
            raise ChainParseError(f"unexpected token {tok!r}")
        # a handle, with an optional `@tool@tool` grant suffix (PR-9b)
        handle = self._next()
        base = self._tasks.get(handle)
        if base is None:
            raise UnknownHandleError(f"unknown task handle '{handle}'")
        tags = self._take_grants()
        return self._node_for(handle, base, tags)

    def _take_grants(self) -> List[str]:
        """Consume a ``grants := ("@" handle)+`` suffix (PR-9b): order-preserving
        deduped tool names. A stray ``@`` with no tool name is a parse error."""
        tags: List[str] = []
        while self._peek() == "@":
            self._next()  # consume the `@`
            nxt = self._peek()
            if nxt is None or nxt in ("|", "&", ">", "[", "]", "@"):
                raise ChainParseError("unexpected token after '@' (expected a tool name)")
            tag = self._next()
            if tag not in tags:
                tags.append(tag)
        return tags

    def _node_for(self, handle: str, base: Task, tags: List[str]) -> Task:
        """Return the AST node for ``handle`` (one COPY per handle so reuse is one
        node + the caller's ``tasks`` are never mutated), merging any ``@``-grants
        (version ``"1"``) into a MODEL step's tool_contract; a non-model grant is a
        fail-closed authoring error."""
        node = self._nodes.get(handle)
        if node is None:
            node = Task(
                replace(
                    base.step,
                    tool_contract=dict(base.step.tool_contract),
                    params=dict(base.step.params),
                )
            )
            self._nodes[handle] = node
        if tags:
            if node.step.kind != "model":
                raise AgenticStepError(
                    f"`@` tool grants on a non-model step '{handle}' "
                    f"(kind '{node.step.kind}'); `@tool` tags require a model step"
                )
            for tag in tags:
                node.step.tool_contract.setdefault(tag, "1")
        return node


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
        if ch in "|&>[]@":
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


def chain(
    expr: str,
    tasks: Dict[str, Task],
    *,
    seed: int = 0,
    context: Optional[Sequence[str]] = None,
) -> Chain:
    """Parse a chain string expression (the exact grammar in SPEC.md), resolving
    handles via ``tasks``, into a :class:`Chain`.

    ``context`` is an optional list of context-bundle handles to attach (PR-7b) —
    chain-level grounding the server injects into every entry Mote (see
    :meth:`Chain.context`); also settable fluently via ``.context(...)``.

    Raises :class:`ChainParseError` on an empty/malformed expression or empty
    group, :class:`UnknownHandleError` on a handle absent from ``tasks``, and
    (at lowering time) :class:`ChainCycleError` on a cycle / self-loop. Tasks
    defined but unused are ignored.

    >>> from kortecx.chains import chain, model, pure
    >>> tasks = {"a": model(prompt="research"), "b": pure()}
    >>> low = chain("a > b", tasks).lowering()
    >>> [s["kind"] for s in low["steps"]]
    ['model', 'pure']
    >>> low["edges"]
    [{'parent': 0, 'child': 1, 'edge': 'data'}]

    ``&`` / ``|`` fan tasks in parallel (no edge); ``[ ]`` groups; ``handle@tool``
    tags a MODEL step with a bounded tool grant (order-preserving, deduped):

    >>> agentic = chain("m@web-search@web-search", {"m": model(prompt="go")})
    >>> agentic.lowering()["steps"][0]["tool_contract"]
    {'web-search': '1'}
    >>> chain("a > a", {"a": model(prompt="x")}).lowering()  # doctest: +IGNORE_EXCEPTION_DETAIL
    Traceback (most recent call last):
    ...
    kortecx.chains.ChainCycleError: chain has a cycle
    """
    if not expr.strip():
        raise ChainParseError("empty chain expression")
    root = _Parser(expr, tasks).parse()
    return Chain(root, seed=seed, context_bundles=context)
