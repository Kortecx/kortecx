---
id: agent-runner
title: Agent runner
sidebar_label: Agent runner
description: The live agent loop — plan, re-plan, critic gates, and ReAct turns with tools.
---

# Agent runner

The live agent loop runs **inside `kx serve`** and is crash-safe end to end:
models **plan** topology, **re-plan** on failure, pass **critic** gates, and run
**ReAct turns with real MCP tools** — every turn a durable fact. Crash the server
mid-loop and it resumes from its committed turns.

:::note Coming soon
The agent-runner guide — driving the loop, budgets (`max_turns` / `max_tool_calls`),
re-plan and critic semantics, and consuming a run via `ListReactTurns` — lands
with a later docs PR. For now, see the
[Quickstart agent loop](./quickstart.md#run-the-agent-loop) and
[Concepts → ReAct chain](./concepts.md#react-chain--reactround).
:::
