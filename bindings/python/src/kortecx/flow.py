"""The fluent Flow builder — the headline authoring surface (Batch V2).

```python
import kortecx as kx

out = (kx.flow()
       .agent("Research the topic", tools=["web-search"])
       .then("Critique the findings")
       .run())
print(out.text)
```

A thin, discoverable veneer over the operator AST in :mod:`kortecx.chains`: every
method appends to the SAME ``_Seq`` / ``_Par`` node graph the ``>>`` / ``&`` / ``|``
operators build, so a :class:`Flow` lowers **byte-identically** to the equivalent
chain (the golden-corpus tri-surface contract holds by construction — a Flow is sugar,
never a new wire shape). Defaults are filled in (served model, budget 8/6, wait) so the
common case is one line; every knob stays optional. SN-8: a Flow describes TOPOLOGY
only — the server compiles + warrants every step.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, List, Optional, Union

from .chains import (
    Chain,
    ChainError,
    Task,
    _as_node,
    _Node,
    _Par,
    _Seq,
)
from .chains import model as _model
from .chains import pure as _pure
from .chains import tool as _tool

if TYPE_CHECKING:
    from .run import Result
    from .v1 import gateway_pb2 as _g

#: Anything a builder method can fold into the graph: a prompt (⇒ an agent step), a
#: :class:`~kortecx.chains.Task`, another :class:`Flow`, or a raw operator node.
FlowItem = Union[str, Task, "Flow", _Seq, _Par]


def _to_node(item: "FlowItem") -> "_Node":
    """Resolve a flow item to an operator AST node. A bare ``str`` is an agent
    (MODEL) step with all-default config — the most common case."""
    if isinstance(item, Flow):
        return item._require_node()
    if isinstance(item, str):
        return _model(prompt=item)
    return _as_node(item)


class Flow:
    """A fluent chain builder. Each builder method APPENDS to the graph and returns
    ``self`` (chain the calls); terminate with :meth:`run` / :meth:`submit` /
    :meth:`to_chain`. The string DSL (:func:`~kortecx.chains.chain`) and operator
    sugar (``a >> b``) remain available as power forms — all three lower identically.
    """

    def __init__(self, *, seed: int = 0) -> None:
        self._node: Optional[_Node] = None
        self._seed = seed
        self._context: List[str] = []

    # -- builders (each appends SEQUENTIALLY after the current tail) --

    def _seq_append(self, node: "_Node") -> "Flow":
        self._node = node if self._node is None else _Seq([_as_node(self._node), node])
        return self

    def agent(
        self,
        prompt: str,
        *,
        tools=None,
        model: str = "",
        max_turns: Optional[int] = None,
        max_tool_calls: Optional[int] = None,
        reasoning: Optional[str] = None,
    ) -> "Flow":
        """Append an agent (MODEL) step. ``model`` defaults to the served model (the
        client's ``default_model`` fills a blank one at submit, SN-8); pass ``tools``
        to make it a deterministic-agentic step — a bounded reason→tool→observe loop
        over the granted SET (PR-9b; the execution lane lights up with PR-9b-2)."""
        return self._seq_append(
            _model(
                model,
                prompt,
                tools=tools,
                max_turns=max_turns,
                max_tool_calls=max_tool_calls,
                reasoning=reasoning,
            )
        )

    def step(self, **params: Union[bytes, str]) -> "Flow":
        """Append a PURE step (deterministic, no model/egress)."""
        return self._seq_append(_pure(**params))

    def tool(self, tool_id: str, tool_version: str = "1", **args: object) -> "Flow":
        """Append a standalone TOOL step — fire ONE registered tool (PR-6b-2). The
        server resolves it in its live registry + builds the warrant (SN-8)."""
        return self._seq_append(_tool(tool_id, tool_version, **args))

    def then(self, item: "FlowItem", **agent_kwargs: object) -> "Flow":
        """Append ``item`` sequentially. A bare string is an agent step (with the
        optional ``agent_kwargs``, e.g. ``tools=`` / ``reasoning=``); a Task or Flow
        is appended as-is. Reads as the natural follow-on after :meth:`agent`."""
        if isinstance(item, str):
            return self.agent(item, **agent_kwargs)  # type: ignore[arg-type]
        return self._seq_append(_to_node(item))

    def parallel(self, *items: "FlowItem") -> "Flow":
        """Append a PARALLEL fan of ``items`` (each a prompt / Task / Flow) as one
        merge node, sequential after the current tail — a fan-out when something
        precedes it, a fan-in when something follows (``a > [b & c]`` / ``[a & b] > c``)."""
        if not items:
            raise ChainError("parallel() needs at least one branch")
        return self._seq_append(_Par([_to_node(i) for i in items]))

    def context(self, *handles: str) -> "Flow":
        """Attach context-bundle handles to the run (PR-7, chain-level grounding the
        server injects into every entry Mote at bind, SN-8). Appends in order."""
        self._context.extend(handles)
        return self

    # -- terminals --

    def _require_node(self) -> "_Node":
        if self._node is None:
            raise ChainError("empty flow — add a step (.agent / .step / .tool) first")
        return self._node

    def to_chain(self) -> Chain:
        """Lower this flow to a :class:`~kortecx.chains.Chain` (the operator/DSL form)."""
        return Chain(self._require_node(), seed=self._seed, context_bundles=self._context)

    def build(self):
        """Lower to a ``SubmitWorkflowRequest`` (via :meth:`to_chain`)."""
        return self.to_chain().build()

    def lowering(self):
        """The canonical pre-encoding lowering (the corpus-parity dict)."""
        return self.to_chain().lowering()

    def run(
        self, *, wait: bool = True, timeout: float = 120.0, client=None
    ) -> "Union[_g.RunHandle, Result]":
        """Submit and (by default) WAIT for the committed :class:`~kortecx.run.Result`,
        over the given ``client`` or the zero-config default client. ``wait=False``
        returns a run handle."""
        from .defaults import default_client

        kx = client if client is not None else default_client()
        return kx.run_chain(self.to_chain(), wait=wait, timeout=timeout)

    def submit(self, *, client=None) -> "_g.RunHandle":
        """Submit without waiting — return the run handle."""
        from .defaults import default_client

        kx = client if client is not None else default_client()
        handle = kx.run_chain(self.to_chain(), wait=False)
        return handle  # type: ignore[return-value]


def flow(*, seed: int = 0) -> Flow:
    """Start a fluent chain: ``kx.flow().agent(...).then(...).run()``. The headline
    authoring surface — reads top-to-bottom, IDE-discoverable, defaults filled in."""
    return Flow(seed=seed)
