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


class Agent:
    """A reusable agent: instructions + an optional tool set + model/loop config.
    Call :meth:`run` with a task. Frozen (default) or ``dynamic=True`` (the react lane)."""

    def __init__(
        self,
        instructions: str = "",
        *,
        tools: "Optional[Union[Sequence[str], Mapping[str, str]]]" = None,
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
        """Run ``task``. Frozen lane (default) ⇒ a single agent step; ``dynamic=True``
        ⇒ the steered ``kx/recipes/react`` recipe. Waits for the committed
        :class:`~kortecx.run.Result` unless ``wait=False``."""
        from .defaults import default_client

        kx = client if client is not None else default_client()
        if self.dynamic:
            args = {"instruction": self._prompt(task)}
            return kx.invoke(REACT_RECIPE_HANDLE, args, wait=wait, timeout=timeout)
        return self.as_flow(task).run(wait=wait, timeout=timeout, client=kx)
