"""Curated **personas** — reusable, named agent instruction sets.

```python
import kortecx as kx

# a persona is just an Agent preset (its curated instructions become the step prompt)
kx.swarm(kx.persona("researcher"), kx.persona("critic"), kx.persona("writer"),
         goal="Write a briefing on durable execution").run()
```

A persona is a curated **instruction string** with a stable name. :func:`persona`
returns an :class:`~kortecx.agent.Agent` whose ``instructions`` are that string, so a
persona composes anywhere an Agent does — a swarm participant, a reusable
``persona("researcher").on("topic A")`` flow, or a standalone ``.run(task)``.

Personas are **identity-bearing, not presentation-only**: the instruction text folds
into the agent step's ``config_subset[PROMPT_KEY]`` (like any prompt), so two agents
that differ only by persona are genuinely distinct, replayable Motes — the same
persona + task always re-derives the same ``MoteId``. This is purely client-side
authoring sugar (a curated ``{name: instructions}`` table); the SERVER still compiles
+ warrants every step (SN-8), and the canonical projection digest is unaffected (the
demo uses no persona — a persona only ever changes NEW motes).

The library is intentionally small + provider-neutral; pass your own string to
``Agent(instructions=...)`` for anything bespoke. ``persona(name, tools=[...])`` layers
a tool set onto the role (making the step a bounded reason→tool→observe agent).
"""

from __future__ import annotations

from typing import TYPE_CHECKING, List, Mapping, Optional, Sequence, Union

if TYPE_CHECKING:
    from .agent import Agent

#: The curated persona library: a stable ``name → instructions`` map. The strings are
#: role framings (not tasks) — a swarm/`.on()` supplies the concrete task. Kept concise
#: so they steer without dominating a small local model's context window.
PERSONAS: "dict[str, str]" = {
    "researcher": (
        "You are a meticulous researcher. Gather the relevant facts, cite concrete "
        "evidence, separate what is known from what is inferred, and flag gaps or "
        "uncertainty explicitly. Prefer primary detail over generalities."
    ),
    "analyst": (
        "You are a rigorous analyst. Break the problem into parts, reason step by step, "
        "quantify where you can, and state the assumptions behind each conclusion. Call "
        "out the strongest and weakest points of your own analysis."
    ),
    "critic": (
        "You are a sharp, fair critic. Find the flaws, unstated assumptions, edge cases, "
        "and failure modes in the material under review. Be specific and constructive: "
        "for each problem, say why it matters and what would fix it."
    ),
    "skeptic": (
        "You are a disciplined skeptic. Challenge every claim: ask what evidence supports "
        "it, what would falsify it, and where it could be wrong. Do not accept a "
        "conclusion until it survives scrutiny; say plainly when it does not."
    ),
    "planner": (
        "You are a decisive planner. Turn the goal into an ordered, concrete plan: the "
        "steps, their dependencies, the owner or tool for each, and the risks. Prefer the "
        "simplest plan that achieves the goal; make the sequencing explicit."
    ),
    "strategist": (
        "You are a strategist. Consider the options, their trade-offs, and second-order "
        "effects, then recommend one course of action with the reasoning behind it. Be "
        "explicit about what you are optimizing for and what you are trading away."
    ),
    "engineer": (
        "You are a pragmatic engineer. Produce correct, minimal, maintainable solutions; "
        "handle edge cases and failure paths; and explain the key design decisions. "
        "Prefer clarity over cleverness and state the assumptions you relied on."
    ),
    "writer": (
        "You are a clear, precise writer. Turn the material into well-structured prose "
        "with a strong through-line: lead with the point, support it concisely, and cut "
        "filler. Match the tone to the audience; never invent facts."
    ),
    "editor": (
        "You are a careful editor. Tighten the writing for clarity, accuracy, and flow "
        "without changing the meaning. Fix structure, remove redundancy, and flag any "
        "claim that is unsupported or ambiguous."
    ),
    "summarizer": (
        "You are a faithful summarizer. Distill the material to its essential points in "
        "the fewest words that preserve meaning. Keep the load-bearing details, drop the "
        "rest, and never introduce information that was not present."
    ),
}


def persona_names() -> "List[str]":
    """The sorted names in the curated persona library."""
    return sorted(PERSONAS)


def persona(
    name: str,
    *,
    tools: "Optional[Union[Sequence[object], Mapping[str, str]]]" = None,
    model: str = "",
    max_turns: "Optional[int]" = None,
    max_tool_calls: "Optional[int]" = None,
    reasoning: "Optional[str]" = None,
    dynamic: bool = False,
) -> "Agent":
    """Return an :class:`~kortecx.agent.Agent` preset with the curated ``name`` role.

    Layer a tool set with ``tools=[...]`` to make the persona a bounded
    reason→tool→observe agent. Compose it directly (``kx.swarm(persona("critic"),
    ...)``), bind a task (``persona("researcher").on("topic")``), or run it standalone
    (``persona("writer").run("draft the intro")``). Raises :class:`KeyError` for an
    unknown name — pass ``Agent(instructions=...)`` with your own string for a bespoke
    role."""
    from .agent import Agent

    if name not in PERSONAS:
        raise KeyError(
            f"unknown persona {name!r} — known: {persona_names()} "
            "(or use Agent(instructions=...) with your own role)"
        )
    return Agent(
        PERSONAS[name],
        tools=tools,
        model=model,
        max_turns=max_turns,
        max_tool_calls=max_tool_calls,
        reasoning=reasoning,
        dynamic=dynamic,
    )
