"""A first-class Agent (Batch V2).

```python
import kortecx as kx

analyst = kx.Agent("You are a research analyst.", tools=["web-search", "fs-list"])
print(analyst.run("Summarize the kortecx README").text)
```

Two lanes (D161), one object — the API name mirrors the distinction:

- **Default = deterministic / frozen** — a single agent step with a FIXED tool-grant
  SET (replayable; the tool set is part of the step's identity). Pure client sugar over
  :class:`~kortecx.flow.Flow`. The frozen tool-EXECUTION lights up with PR-9b-2; until
  then a *tool-bearing* frozen agent is refused server-side at submit — use
  ``dynamic=True`` (or a standalone ``tool()`` step) for tool-calling today.
- ``dynamic=True`` → the **steered** ``kx/recipes/react`` recipe, where the model picks
  tools turn by turn. Works today.

SN-8: an Agent describes intent only — the server compiles + warrants every step.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Mapping, Optional, Sequence, Union

from .flow import flow as _flow

if TYPE_CHECKING:
    from .client import KxClient
    from .flow import Flow
    from .run import Result, Run
    from .v1 import gateway_pb2 as _g

#: The steered, dynamic-tool recipe (the model chooses tools turn by turn).
REACT_RECIPE_HANDLE = "kx/recipes/react"
#: The steered lane that AUTO-GRANTS the live registered tool set (PR-6b-4) — the
#: dynamic lane routes here when the agent carries tools (only react-auto can fire a
#: dialed/registered tool). Requires the serve to run with ``KX_SERVE_AUTOGRANT=1``.
REACT_AUTO_RECIPE_HANDLE = "kx/recipes/react-auto"


class Agent:
    """A reusable agent: instructions + an optional tool set + model/loop config.
    Call :meth:`run` with a task. Frozen (default) or ``dynamic=True`` (the react lane)."""

    def __init__(
        self,
        instructions: str = "",
        *,
        tools: "Optional[Union[Sequence[object], Mapping[str, str]]]" = None,
        model: str = "",
        max_turns: Optional[int] = None,
        max_tool_calls: Optional[int] = None,
        reasoning: Optional[str] = None,
        dynamic: bool = False,
    ) -> None:
        self.instructions = instructions
        self.tools = tools
        self.model = model
        self.max_turns = max_turns
        self.max_tool_calls = max_tool_calls
        self.reasoning = reasoning
        self.dynamic = dynamic

    def _prompt(self, task: str) -> str:
        """Compose the per-call instruction = the standing instructions + the task."""
        return f"{self.instructions}\n\n{task}".strip() if self.instructions else task

    def as_flow(self, task: str) -> "Flow":
        """The FROZEN-lane :class:`~kortecx.flow.Flow` for ``task`` — a single agent
        step carrying this agent's config. (The dynamic lane runs a recipe, not a flow.)"""
        return _flow().agent(
            self._prompt(task),
            tools=self.tools,
            model=self.model,
            max_turns=self.max_turns,
            max_tool_calls=self.max_tool_calls,
            reasoning=self.reasoning,
        )

    def run(
        self,
        task: str,
        *,
        wait: bool = True,
        timeout: float = 120.0,
        client: "Optional[KxClient]" = None,
    ) -> "Union[_g.RunHandle, Run, Result]":
        """Run ``task``.

        - **frozen lane (default)** ⇒ a single agent step. A tool-bearing frozen agent
          runs a deterministic-agentic loop that **lands in PR-9b-2** (refused at
          submit today) — so a clear pre-flight hint is raised; use ``dynamic=True`` or
          ``flow().tool(fn, **args)`` to call tools today.
        - ``dynamic=True`` ⇒ the steered react lane. With tools it routes to
          ``kx/recipes/react-auto`` (the only lane that fires registered/dialed tools;
          needs ``KX_SERVE_AUTOGRANT=1``); without tools, plain ``kx/recipes/react``.

        Waits for the committed :class:`~kortecx.run.Result` unless ``wait=False``."""
        from .defaults import default_client

        kx = client if client is not None else default_client()
        has_tools = bool(self.tools)
        if self.dynamic:
            # The react / react-auto recipes REQUIRE the bounded-loop budget (the
            # `react_contract` slots; the UI's planReactArgs mirrors this) — default
            # to the recipe's anchored 8 / 6 when the agent didn't set them.
            args = {
                "instruction": self._prompt(task),
                "max_turns": self.max_turns if self.max_turns is not None else 8,
                "max_tool_calls": self.max_tool_calls if self.max_tool_calls is not None else 6,
            }
            if not has_tools:
                return kx.invoke(REACT_RECIPE_HANDLE, args, wait=wait, timeout=timeout)
            from .errors import KxNotFound
            from .tools import ToolError, register_tools

            register_tools(kx, self.tools)
            try:
                return kx.invoke(REACT_AUTO_RECIPE_HANDLE, args, wait=wait, timeout=timeout)
            except KxNotFound as exc:
                raise ToolError(
                    "the dynamic tool lane needs the 'kx/recipes/react-auto' recipe — "
                    "serve with KX_SERVE_AUTOGRANT=1 to enable it (it auto-grants the "
                    "registered tool set to the loop)"
                ) from exc
        if has_tools:
            from .tools import ToolError

            raise ToolError(
                "a frozen Agent with a tool set runs a deterministic-agentic loop that "
                "lands in PR-9b-2 and is refused at submit today; use "
                "Agent(..., dynamic=True) for the steered react lane, or "
                "flow().tool(fn, **args) to fire one tool deterministically"
            )
        return self.as_flow(task).run(wait=wait, timeout=timeout, client=kx)
