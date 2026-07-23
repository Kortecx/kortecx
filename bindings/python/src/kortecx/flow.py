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
never a new wire shape). Defaults are filled in (served model, budget 8/20, wait) so the
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
    from .run import Result, Run

#: Anything a builder method can fold into the graph: a prompt (⇒ an agent step), a
#: :class:`~kortecx.chains.Task`, another :class:`Flow`, or a raw operator node.
FlowItem = Union[str, Task, "Flow", _Seq, _Par]

#: AGENTIC-VISION: the step-config key a :meth:`Flow.image` ref binds into (the SAME key
#: the vision/react-vision recipes publish + the gateway executor / coordinator read).
IMAGE_REF_KEY = "image_ref"


def _to_node(item: "FlowItem") -> "_Node":
    """Resolve a flow item to an operator AST node. A bare ``str`` is an agent
    (MODEL) step with all-default config — the most common case."""
    if isinstance(item, Flow):
        return item._require_node()
    if isinstance(item, str):
        return _model(prompt=item)
    return _as_node(item)


#: The default synthesizer prompt for :meth:`Flow.swarm` / :meth:`Flow.team` — a MODEL
#: gather step reads every parallel participant's committed output (injected as its
#: Data-edge parents, F-7) and merges them.
_DEFAULT_SWARM_GATHER = (
    "You are the lead. Synthesize the parallel agents' results above into one "
    "coherent, complete answer. Reconcile disagreements, keep what is well-supported, "
    "and drop redundancy."
)
_DEFAULT_FAN_GATHER = "Combine the parallel results above into one coherent answer."
_DEFAULT_REDUCE = "Reduce the mapper results above into one consolidated result."

#: The default lead/planner prompt for :meth:`Flow.supervisor` — a MODEL step that reads
#: the goal, decomposes it, and (via its committed output on each worker's Data edge)
#: steers the team. Workers run on the plan; the supervisor gather integrates their results.
_DEFAULT_SUPERVISOR_PLANNER = (
    "You are the supervisor. Break the task into clear, independent subtasks for the "
    "team and state each subtask precisely, so each teammate knows exactly what to do."
)
#: The default integrator prompt for :meth:`Flow.supervisor` — the lead reads every
#: worker's committed output (its Data-edge parents, F-7) and produces one final answer.
_DEFAULT_SUPERVISOR_GATHER = (
    "You are the supervisor. Integrate the team's results above into one complete, "
    "coherent answer. Reconcile disagreements, keep what is well-supported, drop redundancy."
)

#: The default judge prompt for :meth:`Flow.consensus` (``vote="judge"``) — a MODEL step
#: that SELECTS the single best candidate (distinct from :meth:`Flow.swarm`, which MERGES).
_DEFAULT_CONSENSUS_JUDGE = (
    "You are the judge. Read the candidate answers above and choose the single best one; "
    "reply with that answer verbatim, without merging or editing the candidates."
)
#: The ``config_subset`` key (mirrors ``kx_mote::CONSENSUS_VOTE_KEY``) marking a PURE sink
#: as an exact-equality consensus vote — the server reduces its parents to the plurality
#: winner (SN-8: exact byte-equality, ties → first-appearance). Only ``"majority"`` today.
_CONSENSUS_VOTE_KEY = "kx.consensus.vote"

#: The default reviewer prompt for :meth:`Flow.review_loop` — each pass reviews the
#: previous output for errors/gaps and emits an improved version.
_DEFAULT_REVIEW = (
    "Review the work above for errors, gaps, and weaknesses, then output an improved "
    "version that fixes them. Reply with only the improved work."
)


def _join_goal(text: str, goal: str) -> str:
    """Compose a participant prompt = its role/prompt + the shared ``goal`` (if any)."""
    return f"{text}\n\n{goal}".strip() if goal else text


def _participant_to_node(item: "object", goal: str) -> "_Node":
    """Resolve ONE swarm/team participant to an agentic-leaf AST node.

    Accepts a prompt ``str``, a ``(prompt, tools)`` tuple, a :class:`Flow` /
    :class:`~kortecx.chains.Task` (already task-bound — ``goal`` ignored), or an
    :class:`~kortecx.agent.Agent` / :func:`~kortecx.personas.persona` (duck-typed on
    ``_prompt`` to avoid the ``agent → flow`` import cycle). A persona/Agent with a
    tool set lowers to a bounded reason→tool→observe leaf."""
    if isinstance(item, Flow):
        return item._require_node()
    if isinstance(item, (Task, _Seq, _Par)):
        return item
    if isinstance(item, tuple):
        prompt = str(item[0])
        tools = item[1] if len(item) > 1 else None
        return _model(prompt=_join_goal(prompt, goal), tools=tools)
    if isinstance(item, str):
        return _model(prompt=_join_goal(item, goal))
    prompt_fn = getattr(item, "_prompt", None)
    if callable(prompt_fn):  # duck-typed Agent / persona
        # model_id is the FIRST positional arg of chains.model, prompt the SECOND — pass
        # them positionally (as Flow.agent does); a keyword `model=` would leak into
        # **params (chains.model has no `model` kwarg). The persona/Agent instructions +
        # goal are the PROMPT, never the model_id.
        return _model(
            getattr(item, "model", "") or "",
            prompt_fn(goal),
            tools=getattr(item, "tools", None),
            max_turns=getattr(item, "max_turns", None),
            max_tool_calls=getattr(item, "max_tool_calls", None),
            reasoning=getattr(item, "reasoning", None),
        )
    raise ChainError(
        f"not a swarm participant: {item!r} — pass a prompt, (prompt, tools), an "
        "Agent/persona, or a Flow"
    )


def _sink_node(
    gather: "Optional[Union[str, FlowItem]]", synthesize: bool, default_prompt: str
) -> "_Node":
    """Build the fan-in sink: a MODEL synthesizer (``gather`` is a prompt string, or the
    ``default_prompt`` when ``synthesize`` and no explicit sink), an explicit sink node
    (a Task/Flow ``gather``), or a PURE deterministic fold (``synthesize=False``)."""
    if isinstance(gather, str):
        return _model(prompt=gather)
    if gather is not None:
        return _to_node(gather)
    if synthesize:
        return _model(prompt=default_prompt)
    return _as_node(_pure())


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
        #: Connectors to register (each a ``register_mcp_server`` kwargs dict) BEFORE
        #: this flow submits — see :meth:`with_mcp`. Stored OFF the lowered graph so
        #: ``to_chain`` / ``build`` stay byte-identical (the golden digest holds).
        self._mcp: List[dict] = []
        #: RC5a: durable memory facts to REMEMBER (each a ``store_memory`` kwargs dict)
        #: BEFORE this flow submits — see :meth:`with_memory`. Stored OFF the lowered
        #: graph so ``to_chain`` / ``build`` stay byte-identical (the golden digest holds).
        self._memory: List[dict] = []
        #: AGENTIC-VISION: an image ref pending for the NEXT agent step (set by
        #: :meth:`image`, consumed + cleared by :meth:`agent`). Per-step, so a multi-step
        #: flow can ground each step with a different image.
        self._pending_image: Optional[str] = None

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
        skills=None,
        connections=None,
        datasets=None,
    ) -> "Flow":
        """Append an agent (MODEL) step. ``model`` defaults to the served model (the
        client's ``default_model`` fills a blank one at submit, SN-8); pass ``tools``
        to make it a deterministic-agentic step — a bounded reason→tool→observe loop
        over the granted SET (PR-9b; the execution lane is LIVE as of PR-9b-2).

        ``skills`` / ``connections`` / ``datasets`` are APP-ONLY per-node capability
        bindings — the catalog capabilities THIS step uses when the flow becomes an App
        (``app(...)``). They bind to this node, not the whole App; on the plain workflow
        path they are refused (no ``references`` to name into).

        AGENTIC-VISION: a preceding :meth:`image` grounds this step — the served VLM reasons
        over that image on every turn of the step's loop (the ref binds into the step's
        ``config_subset[image_ref]``)."""
        image = self._pending_image
        self._pending_image = None
        extra = {IMAGE_REF_KEY: image} if image is not None else {}
        return self._seq_append(
            _model(
                model,
                prompt,
                tools=tools,
                max_turns=max_turns,
                max_tool_calls=max_tool_calls,
                reasoning=reasoning,
                skills=skills,
                connections=connections,
                datasets=datasets,
                **extra,
            )
        )

    def image(self, ref: str) -> "Flow":
        """AGENTIC-VISION: attach an image to the NEXT agent step. ``ref`` is a 64-hex
        content ref — upload the bytes once via ``client.put_content(data).content_ref``,
        then ground one or more agent steps with it. The served VLM reasons over the image
        on EVERY turn of that step's loop (durably carried across the chain). Per-step: a
        later ``.image()`` before another ``.agent()`` grounds that step with a different
        image. Lowers client-free + deterministically (the golden tri-surface contract)."""
        self._pending_image = ref
        return self

    def step(self, **params: Union[bytes, str]) -> "Flow":
        """Append a PURE step (deterministic, no model/egress)."""
        return self._seq_append(_pure(**params))

    def tool(
        self, tool_id: "Union[str, object]", tool_version: str = "1", **args: object
    ) -> "Flow":
        """Append a standalone TOOL step — fire ONE tool (PR-6b-2). The server
        resolves it in its live registry + builds the warrant (SN-8).

        ``tool_id`` is either a registered tool's name OR a ``@kx.tool``-decorated
        LOCAL function (V2b) — the SDK registers the function as a stdio MCP server
        at the run terminal and fires the resolved tool deterministically."""
        from .tools import local_tool_def, local_tool_node

        tdef = local_tool_def(tool_id)
        if tdef is not None:
            return self._seq_append(local_tool_node(tdef, args))
        return self._seq_append(_tool(tool_id, tool_version, **args))  # type: ignore[arg-type]

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

    def with_mcp(
        self,
        name: str,
        *,
        transport: str = "stdio",
        endpoint: str,
        args: Optional[List[str]] = None,
        tls_required: bool = False,
        credential_ref: str = "",
        session_mode: str = "stateless",
    ) -> "Flow":
        """Register an external MCP **connector** at run time, BEFORE this flow
        submits, so its namespaced ``<name>/<tool>`` tools resolve for a downstream
        ``.agent(tools=[...])`` / ``.tool(...)`` — connectors are thus reachable from
        the SAME single chaining entry point as everything else::

            (kx.flow()
               .with_mcp("fs", endpoint="npx",
                         args=["-y", "@modelcontextprotocol/server-filesystem", "/data"])
               .agent("list /data", tools=["fs/list_directory"])
               .run())

        Pure pre-submit sugar over :meth:`KxClient.register_mcp_server` (same args; a
        connector = an external MCP server, see ``kx-extension-sdk``). It does NOT
        change the lowered workflow — :meth:`to_chain` / :meth:`build` are
        byte-identical with or without it, so the golden tri-surface digest holds;
        registration is an imperative side effect, never a DAG node. Idempotent
        (server-derived id + upsert), so re-running the flow is safe. ``credential_ref``
        names an env var / vault key — the secret VALUE never travels (D81)."""
        self._mcp.append(
            {
                "name": name,
                "transport": transport,
                "endpoint": endpoint,
                "args": list(args or []),
                "tls_required": tls_required,
                "credential_ref": credential_ref,
                "session_mode": session_mode,
            }
        )
        return self

    def _register_mcp(self, kx) -> None:
        """Register each :meth:`with_mcp` connector (in declaration order) before the
        flow submits, so referenced ``<name>/<tool>`` tools resolve at compile."""
        for spec in self._mcp:
            kx.register_mcp_server(**spec)

    def with_memory(self, facts: "Union[str, List[str]]") -> "Flow":
        """Seed durable MEMORY facts (RC5a), BEFORE this flow submits, so a downstream
        ``.agent(...)`` on a ``kx/recipes/react-memory`` chain can ``recall`` them —
        memory is thus reachable from the SAME single chaining entry point as
        everything else::

            (kx.flow()
               .with_memory(["the deadline is March 3rd", "the client prefers email"])
               .agent("when is my deadline?")
               .run())

        Pure pre-submit sugar over :meth:`KxClient.store_memory` (content-addressed +
        idempotent). It does NOT change the lowered workflow — :meth:`to_chain` /
        :meth:`build` are byte-identical with or without it, so the golden tri-surface
        digest holds; the store is an imperative side effect, never a DAG node.
        Every memory is scoped to the caller's own principal."""
        for fact in [facts] if isinstance(facts, str) else facts:
            self._memory.append({"content": fact})
        return self

    def _register_memory(self, kx) -> None:
        """Store each :meth:`with_memory` fact (in declaration order) before the flow
        submits, so a downstream ``recall`` in a react-memory chain can surface it."""
        for spec in self._memory:
            kx.store_memory(**spec)

    # -- orchestration (parallel agentic patterns; pure client composition) --

    def swarm(
        self,
        *agents: "object",
        goal: str = "",
        gather: "Optional[Union[str, FlowItem]]" = None,
        synthesize: bool = True,
    ) -> "Flow":
        """Fan out to N parallel agents, then gather (a **swarm**). Each ``agent`` is a
        prompt, a ``(prompt, tools)`` tuple, an :class:`~kortecx.agent.Agent` /
        :func:`~kortecx.personas.persona`, or a :class:`Flow`; they run **concurrently**
        as independent deterministic-agentic steps (each its own crash-safe, replayable
        salt-2 chain), then a gather step merges their committed outputs::

            (kx.flow()
               .swarm(kx.persona("researcher"), kx.persona("critic"), kx.persona("writer"),
                      goal="Write a briefing on durable execution")
               .run())

        ``goal`` is the shared task each participant works on (appended to its
        role/instructions). By default (``synthesize=True``) the gather is a MODEL step
        that reads every participant's output (injected as its Data-edge parents) and
        writes one coherent answer; pass ``gather="<prompt>"`` to steer that synthesis,
        a Task/Flow for a custom sink, or ``synthesize=False`` for a PURE deterministic
        fold. Pure client-side composition (parallel MODEL leaves → one sink) — no new
        step kind, byte-identical to the equivalent ``[a & b] > g`` chain; the SERVER
        drives + warrants each agent (SN-8)."""
        if not agents:
            raise ChainError("swarm() needs at least one agent")
        leaves = [_participant_to_node(a, goal) for a in agents]
        self.parallel(*leaves)
        return self.then(_sink_node(gather, synthesize, _DEFAULT_SWARM_GATHER))

    def team(
        self,
        *agents: "object",
        goal: str = "",
        gather: "Optional[Union[str, FlowItem]]" = None,
    ) -> "Flow":
        """A **team**: the same topology as :meth:`swarm` with a lead that synthesizes
        (``synthesize=True``). Reads naturally for role-based personas working a shared
        ``goal``. ``team(*a, goal=g)`` ≡ ``swarm(*a, goal=g, synthesize=True)``."""
        return self.swarm(*agents, goal=goal, gather=gather, synthesize=True)

    def fan_out_gather(
        self,
        *branches: "FlowItem",
        gather: "Optional[Union[str, FlowItem]]" = None,
        synthesize: bool = True,
    ) -> "Flow":
        """Fan out to N parallel ``branches`` (each a prompt / Task / Flow), then gather
        their outputs — sample-N-ways-and-combine. Default gather = a MODEL combine
        step; ``gather="<prompt>"`` steers it, a Task/Flow gives a custom sink, and
        ``synthesize=False`` folds deterministically (PURE). Surfaces the recipe
        builder's ``fan_out_gather`` topology as first-class client composition."""
        if not branches:
            raise ChainError("fan_out_gather() needs at least one branch")
        leaves = [_to_node(b) for b in branches]
        self.parallel(*leaves)
        return self.then(_sink_node(gather, synthesize, _DEFAULT_FAN_GATHER))

    def map_reduce(
        self,
        *mappers: "FlowItem",
        reduce: "Optional[Union[str, FlowItem]]" = None,
        synthesize: bool = True,
    ) -> "Flow":
        """Map N ``mappers`` in parallel, then reduce their outputs. Default reduce = a
        MODEL consolidation step; ``reduce="<prompt>"`` steers it, a Task/Flow gives a
        custom reducer, and ``synthesize=False`` reduces deterministically (PURE).
        Surfaces the recipe builder's ``map_reduce`` topology as client composition."""
        if not mappers:
            raise ChainError("map_reduce() needs at least one mapper")
        leaves = [_to_node(m) for m in mappers]
        self.parallel(*leaves)
        return self.then(_sink_node(reduce, synthesize, _DEFAULT_REDUCE))

    def supervisor(
        self,
        *workers: "object",
        planner: "Optional[Union[str, FlowItem]]" = None,
        goal: str = "",
        gather: "Optional[Union[str, FlowItem]]" = None,
        rounds: int = 1,
        pool: "Optional[int]" = None,
        synthesize: bool = True,
    ) -> "Flow":
        """A **hierarchical supervisor**: a lead ``planner`` decomposes the ``goal``, the
        ``workers`` each act on that plan in parallel, then the lead integrates their
        results — the topology ``planner > [workers] > gather``::

            (kx.supervisor(kx.persona("researcher"), kx.persona("writer"),
                           planner="Plan a briefing on durable execution",
                           goal="Cover crash-recovery + exactly-once")
               .run())

        Each ``worker`` is a prompt, a ``(prompt, tools)`` tuple, an
        :class:`~kortecx.agent.Agent` / :func:`~kortecx.personas.persona`, or a
        :class:`Flow` (as in :meth:`swarm`); ``planner`` is the same, defaulting to a
        standard lead prompt. The planner's committed output is a Data-edge parent of
        every worker (they run *on* the plan), and every worker feeds the ``gather`` lead
        (default = a MODEL integrator; steer with ``gather="<prompt>"``, a Task/Flow for a
        custom sink, or ``synthesize=False`` for a PURE fold). Pure client-side composition
        (no new step kind), byte-identical to the equivalent ``p > [a & b] > g`` chain; the
        SERVER drives + warrants each agent (SN-8).

        This supervisor is **static-hierarchical** — a fixed team, authored up front.
        ``rounds`` and ``pool`` are reserved for the runtime **topology shaper** (a planner
        that decides team size/roles at execution time and re-plans each round); they sit in
        the signature so the API is stable when the shaper wires them, but passing
        ``rounds>1`` or ``pool`` raises today rather than silently ignoring it. Local worker
        concurrency is governed by the server worker pool (``kx serve --workers`` /
        ``KX_WORKERS``)."""
        if not workers:
            raise ChainError("supervisor() needs at least one worker")
        if rounds != 1:
            raise ChainError(
                "supervisor(rounds>1) requires the runtime topology shaper, which isn't "
                "wired to this static-hierarchical path; use rounds=1"
            )
        if pool is not None:
            raise ChainError(
                "supervisor(pool=…) requires the runtime topology shaper, which isn't wired "
                "to this path; local worker concurrency is set by the server worker pool "
                "(kx serve --workers / KX_WORKERS)"
            )
        plan = (
            _model(prompt=_join_goal(_DEFAULT_SUPERVISOR_PLANNER, goal))
            if planner is None
            else _participant_to_node(planner, goal)
        )
        leaves = [_participant_to_node(w, goal) for w in workers]
        self.then(plan)
        self.parallel(*leaves)
        return self.then(_sink_node(gather, synthesize, _DEFAULT_SUPERVISOR_GATHER))

    def consensus(
        self,
        *voters: "object",
        vote: str = "judge",
        goal: str = "",
        judge: "Optional[Union[str, FlowItem]]" = None,
    ) -> "Flow":
        """Run N ``voters`` in parallel, then reach **consensus** over their outputs —
        the topology ``[v1 & v2 & …] > reduce``::

            (kx.consensus(kx.persona("analyst"), kx.persona("skeptic"), kx.persona("engineer"),
                          goal="Is this design sound?", vote="judge")
               .run())

        Each ``voter`` is a prompt, a ``(prompt, tools)`` tuple, an
        :class:`~kortecx.agent.Agent` / :func:`~kortecx.personas.persona`, or a
        :class:`Flow` (as in :meth:`swarm`). Two reduce modes:

        - ``vote="judge"`` (default): a MODEL judge reads the candidates and **selects the
          single best** one (distinct from :meth:`swarm`, which *merges*). ``judge="<prompt>"``
          steers the selection, or pass a Task/Flow for a custom judge.
        - ``vote="majority"``: the server reduces to the **exact-equality plurality** — the
          most-frequent voter output by EXACT byte-equality, ties broken by first-appearance
          (SN-8: exact equality only, never a similarity score). Best for CONSTRAINED outputs
          (a label / structured JSON / a tool decision) — free-form prose rarely ties, so
          ``judge`` is the usual mode there.

        Pure client-side composition (no new step kind); the SERVER drives + warrants each
        voter and computes the reduce (SN-8)."""
        if not voters:
            raise ChainError("consensus() needs at least one voter")
        if vote not in ("judge", "majority"):
            raise ChainError(f"consensus(vote=…) must be 'judge' or 'majority', got {vote!r}")
        leaves = [_participant_to_node(v, goal) for v in voters]
        self.parallel(*leaves)
        if vote == "judge":
            return self.then(_sink_node(judge, True, _DEFAULT_CONSENSUS_JUDGE))
        # vote == "majority": a PURE sink the server reduces by exact-equality plurality
        # (config_subset[kx.consensus.vote]="majority").
        return self.then(_as_node(_pure(**{_CONSENSUS_VOTE_KEY: "majority"})))

    def review_loop(
        self,
        worker: "object",
        *,
        reviewer: "Optional[Union[str, FlowItem]]" = None,
        rounds: int = 1,
        goal: str = "",
    ) -> "Flow":
        """A **review loop**: a ``worker`` drafts, then a ``reviewer`` reviews-and-improves
        the draft ``rounds`` times — an iterative refine chain
        ``worker > review > review > …``::

            (kx.review_loop("Draft a launch email",
                            reviewer="Tighten it and fix any errors", rounds=2)
               .run())

        ``worker`` is the initial task (a prompt / ``(prompt, tools)`` / Agent / persona /
        Flow); ``reviewer`` is a review prompt or a critic persona applied each round
        (default: review-and-improve). Each pass reads the previous version (its Data-edge
        parent) and emits a better one; the LAST step's output is the result. Pure
        sequential composition (no new step kind) — the author-static refine loop; a
        runtime-adaptive "revise until a critic passes" loop is the topology-shaper
        follow-on. ``rounds`` ≥ 1."""
        if rounds < 1:
            raise ChainError("review_loop() needs rounds >= 1")
        self.then(_participant_to_node(worker, goal))
        for _ in range(rounds):
            self.then(
                _participant_to_node(reviewer, "")
                if reviewer is not None
                else _model(prompt=_DEFAULT_REVIEW)
            )
        return self

    def as_app(self, name: str, *, version: str = "1"):
        """Promote this Flow to a durable, named :class:`~kortecx.app.App` — the
        EXPLICIT boundary (D177) from ad-hoc authoring to a shareable App that runs via
        ``RunApp`` (server-resolved connections + ``secret_scope`` + skills). Chain the
        App rails on the result::

            (kx.flow().agent("Draft and send a reply", tools=["kx-connector-gmail/send"])
               .as_app("mailer").with_gmail().secrets(["KX_GMAIL_CREDENTIAL"])
               .run(args={"to": "x@y.com"}))

        Naming the App is deliberate: connections / skills / secret scope ride the App
        envelope (not the lowered graph, which stays byte-identical), so a bare
        ``Flow.run()`` — which submits a plain workflow — has no place for them. Any
        :meth:`with_mcp` / :meth:`with_memory` side-channels carry over as pre-run
        registrations."""
        from .app import app as _app

        built = _app(name, version=version, seed=self._seed).blueprint(self)
        built._carry_flow_side_channels(list(self._mcp), list(self._memory))
        return built

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

    def to_blueprint(self):
        """Export this flow as a portable blueprint dict (Batch B; via :meth:`to_chain`)."""
        return self.to_chain().to_blueprint()

    def export(self, path) -> None:
        """Write the portable blueprint JSON to ``path`` (Batch B; via :meth:`to_chain`)."""
        self.to_chain().export(path)

    def run(
        self, *, wait: bool = True, timeout: float = 120.0, client=None
    ) -> "Union[Run, Result]":
        """Submit and (by default) WAIT for the committed :class:`~kortecx.run.Result`,
        over the given ``client`` or the zero-config default client. ``wait=False``
        returns a :class:`~kortecx.run.Run` handle (``.wait()`` / ``.events()``)."""
        from .defaults import default_client

        kx = client if client is not None else default_client()
        self._register_mcp(kx)
        self._register_memory(kx)
        return kx.run_chain(self.to_chain(), wait=wait, timeout=timeout)

    def submit(self, *, client=None) -> "Run":
        """Submit without waiting — return a :class:`~kortecx.run.Run` handle. Drive it
        with ``.wait()`` (the first committed Mote), ``.events()``, or ``.tokens(mote)``."""
        from .defaults import default_client

        kx = client if client is not None else default_client()
        self._register_mcp(kx)
        self._register_memory(kx)
        run = kx.run_chain(self.to_chain(), wait=False)
        return run  # type: ignore[return-value]


def flow(*, seed: int = 0) -> Flow:
    """Start a fluent chain: ``kx.flow().agent(...).then(...).run()``. The headline
    authoring surface — reads top-to-bottom, IDE-discoverable, defaults filled in.

    A :class:`Flow` lowers byte-identically to the equivalent operator/DSL chain, so
    you can inspect the topology it will submit without a server:

    >>> from kortecx.flow import flow
    >>> low = flow().agent("Research the topic").then("Critique it").lowering()
    >>> [s["kind"] for s in low["steps"]]
    ['model', 'model']
    >>> low["edges"]
    [{'parent': 0, 'child': 1, 'edge': 'data'}]
    """
    return Flow(seed=seed)


# -- top-level orchestration factories (a swarm is usually the whole flow) --


def swarm(
    *agents: "object",
    goal: str = "",
    gather: "Optional[Union[str, FlowItem]]" = None,
    synthesize: bool = True,
    seed: int = 0,
) -> Flow:
    """``kx.swarm(...)`` — N parallel agents → gather, as a whole flow. Sugar
    for ``kx.flow(seed=seed).swarm(...)``; see :meth:`Flow.swarm`."""
    return flow(seed=seed).swarm(*agents, goal=goal, gather=gather, synthesize=synthesize)


def team(
    *agents: "object",
    goal: str = "",
    gather: "Optional[Union[str, FlowItem]]" = None,
    seed: int = 0,
) -> Flow:
    """``kx.team(...)`` — a swarm with a lead that synthesizes; see :meth:`Flow.team`."""
    return flow(seed=seed).team(*agents, goal=goal, gather=gather)


def fan_out_gather(
    *branches: "FlowItem",
    gather: "Optional[Union[str, FlowItem]]" = None,
    synthesize: bool = True,
    seed: int = 0,
) -> Flow:
    """``kx.fan_out_gather(...)`` — sample N ways, combine; see :meth:`Flow.fan_out_gather`."""
    return flow(seed=seed).fan_out_gather(*branches, gather=gather, synthesize=synthesize)


def map_reduce(
    *mappers: "FlowItem",
    reduce: "Optional[Union[str, FlowItem]]" = None,
    synthesize: bool = True,
    seed: int = 0,
) -> Flow:
    """``kx.map_reduce(...)`` — map N mappers in parallel, then reduce; see
    :meth:`Flow.map_reduce`."""
    return flow(seed=seed).map_reduce(*mappers, reduce=reduce, synthesize=synthesize)


def supervisor(
    *workers: "object",
    planner: "Optional[Union[str, FlowItem]]" = None,
    goal: str = "",
    gather: "Optional[Union[str, FlowItem]]" = None,
    rounds: int = 1,
    pool: "Optional[int]" = None,
    synthesize: bool = True,
    seed: int = 0,
) -> Flow:
    """``kx.supervisor(...)`` — a lead plans, workers execute in parallel, the lead
    integrates, as a whole flow. Sugar for ``kx.flow(seed=seed).supervisor(...)``; see
    :meth:`Flow.supervisor`."""
    return flow(seed=seed).supervisor(
        *workers,
        planner=planner,
        goal=goal,
        gather=gather,
        rounds=rounds,
        pool=pool,
        synthesize=synthesize,
    )


def consensus(
    *voters: "object",
    vote: str = "judge",
    goal: str = "",
    judge: "Optional[Union[str, FlowItem]]" = None,
    seed: int = 0,
) -> Flow:
    """``kx.consensus(...)`` — N voters in parallel → a consensus reduce (a judge that
    selects best-of-N, or an exact-equality majority), as a whole flow. Sugar for
    ``kx.flow(seed=seed).consensus(...)``; see :meth:`Flow.consensus`."""
    return flow(seed=seed).consensus(*voters, vote=vote, goal=goal, judge=judge)


def review_loop(
    worker: "object",
    *,
    reviewer: "Optional[Union[str, FlowItem]]" = None,
    rounds: int = 1,
    goal: str = "",
    seed: int = 0,
) -> Flow:
    """``kx.review_loop(...)`` — a worker drafts, then a reviewer improves it ``rounds``
    times, as a whole flow. Sugar for ``kx.flow(seed=seed).review_loop(...)``; see
    :meth:`Flow.review_loop`."""
    return flow(seed=seed).review_loop(worker, reviewer=reviewer, rounds=rounds, goal=goal)
