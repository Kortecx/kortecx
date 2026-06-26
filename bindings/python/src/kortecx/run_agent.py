"""The embeddable agent-runner — ``run_agent`` (PR-9c-1).

The headline adoption entry (GR18/D149): give a goal (+ optional context + inputs),
the runtime completes it AGENTICALLY — reasoning, calling permission-gated tools, and
returning a reasoned answer PLUS the AUDITED set of actions it took. A thin wrapper
over ``invoke("kx/recipes/react")`` — NEVER ``SubmitRun`` (BLOCKER #5); the warrant is
always SERVER-DERIVED (SN-8), the client only parameterizes the published recipe.

``inputs`` fold into the goal prompt — the ``kx/recipes/react`` contract has no
structured input slot today (instruction / max_turns / max_tool_calls only); a
structured-inputs slot is a later contract addition.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Mapping, Optional, Sequence, Union, cast

from .agent_result import AgentResult, assemble_actions
from .client import REACT_RECIPE_HANDLE

if TYPE_CHECKING:
    from .client import AsyncKxClient, ImageInput, KxClient
    from .run import AsyncRun, Result, Run

#: The recipe's anchored bounded-loop budget (mirrors Agent + the UI's planReactArgs).
#: T-MULTI-ELEMENT-TOOLCALLS: the tool-call cap rose 6 → 20 (decoupled from max_turns —
#: a turn can fire N tools); overridable per call via ``max_tool_calls``.
_DEFAULT_MAX_TURNS = 8
_DEFAULT_MAX_TOOL_CALLS = 20


def _fold_inputs(goal: str, inputs: Optional[Mapping[str, str]]) -> str:
    """Fold structured ``inputs`` into the goal prompt (no structured recipe slot)."""
    if not inputs:
        return goal
    lines = "\n".join(f"- {k}: {v}" for k, v in inputs.items())
    return f"{goal}\n\nInputs:\n{lines}"


def _args(
    goal: str,
    inputs: Optional[Mapping[str, str]],
    max_tool_calls: int = _DEFAULT_MAX_TOOL_CALLS,
) -> dict:
    return {
        "instruction": _fold_inputs(goal, inputs),
        "max_turns": _DEFAULT_MAX_TURNS,
        "max_tool_calls": max_tool_calls,
    }


def run_agent(
    goal: str,
    *,
    context: Optional[Sequence[str]] = None,
    inputs: Optional[Mapping[str, str]] = None,
    max_tool_calls: int = _DEFAULT_MAX_TOOL_CALLS,
    image: "Optional[ImageInput]" = None,
    wait: bool = True,
    timeout: float = 120.0,
    client: "Optional[KxClient]" = None,
) -> "Union[AgentResult, Run]":
    """Complete ``goal`` agentically and return an :class:`~kortecx.agent_result.AgentResult`
    (the final answer + the audited tool actions). ``context`` = published
    context-bundle handles (PR-7) the server resolves + injects; ``inputs`` fold into
    the prompt. ``max_tool_calls`` bounds the chain's total tool calls (default 20,
    ceiling 20; a turn may fire several at once — T-MULTI-ELEMENT-TOOLCALLS). With
    ``wait=False`` returns the started :class:`~kortecx.run.Run` (assemble the result
    later via ``ListReactTurns``). Uses the process-wide default client unless one is passed.

    Raises :class:`~kortecx.errors.KxRunFailed` if the chain dead-letters (terminal
    failure) and :class:`~kortecx.errors.KxWaitTimeout` if it does not settle in time —
    same as ``invoke(wait=True)``."""
    from .defaults import default_client

    kx = client if client is not None else default_client()
    args = _args(goal, inputs, max_tool_calls)
    handle = REACT_RECIPE_HANDLE
    # AGENTIC-VISION: an attached image binds the image-grounded react recipe (form-gated)
    # so the served VLM reasons over it on every turn; fail-closed when no vision model.
    if image is not None:
        handle, args = kx._bind_react_vision(args, kx._resolve_image_ref(image))
    if not wait:
        return cast("Run", kx.invoke(handle, args, context=context, wait=False))
    # invoke(wait=True) on a react handle always settles to a Result (never a Run).
    result = cast("Result", kx.invoke(handle, args, context=context, wait=True, timeout=timeout))
    # PR-R1: scope the action fetch to THIS invocation's chain (serve's shared journal).
    turns = kx.list_react_turns(
        instance_id=result.instance_id, step_salt=result.react_chain_salt or None
    ).turns
    return AgentResult(
        answer=result.text,
        answer_bytes=result.payload,
        actions=assemble_actions(turns),
        run_handle=result.instance_id,
        instance_id=result.instance_id,
    )


async def run_agent_async(
    goal: str,
    *,
    client: "AsyncKxClient",
    context: Optional[Sequence[str]] = None,
    inputs: Optional[Mapping[str, str]] = None,
    max_tool_calls: int = _DEFAULT_MAX_TOOL_CALLS,
    image: "Optional[ImageInput]" = None,
    wait: bool = True,
    timeout: float = 120.0,
) -> "Union[AgentResult, AsyncRun]":
    """Async mirror of :func:`run_agent`. Requires an explicit
    :class:`~kortecx.client.AsyncKxClient` (there is no async default singleton)."""
    args = _args(goal, inputs, max_tool_calls)
    handle = REACT_RECIPE_HANDLE
    if image is not None:
        handle, args = await client._bind_react_vision(args, await client._resolve_image_ref(image))
    if not wait:
        return cast(
            "AsyncRun",
            await client.invoke(handle, args, context=context, wait=False),
        )
    result = cast(
        "Result",
        await client.invoke(handle, args, context=context, wait=True, timeout=timeout),
    )
    page = await client.list_react_turns(
        instance_id=result.instance_id, step_salt=result.react_chain_salt or None
    )
    return AgentResult(
        answer=result.text,
        answer_bytes=result.payload,
        actions=assemble_actions(page.turns),
        run_handle=result.instance_id,
        instance_id=result.instance_id,
    )
