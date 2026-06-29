---
id: agentic-prompts
title: Agentic prompts & the tool menu
sidebar_label: Agentic prompts
description: How Kortecx frames a ReAct agent — the curated agentic system prompt, the granted-tool menu that lets local models propose tools autonomously, and the serve-level controls.
---

# Agentic prompts & the tool menu

Kortecx runs **local OSS models** (Gemma via Ollama or llama.cpp) as ReAct agents.
For a small model to use tools *reliably*, three things have to line up: it must
**know which tools it has**, **know the call format**, and have its **conversation
in order**. Kortecx assembles all three at dispatch — automatically, off-digest, and
identically across engines.

## What the agent is told

On every tool-eligible ReAct turn the runtime builds the model prompt from:

1. **A curated agentic system contract.** It states the reason → act → observe loop,
   the canonical tool-call envelope, and when to stop and answer in plain text. This
   is what makes a 4B–12B model follow the protocol instead of free-styling.
2. **The granted-tool menu.** The tools granted to *this run* (and only those),
   each rendered with its exact callable name, description, typed inputs, and a
   worked example — so the model proposes a **well-formed** call, not a guess.
3. **The conversation so far.** Prior turns and their tool observations, in **time
   order**, so the model reads what it already did and what each tool returned.

The model then proposes a call as:

```json
{"tool_call":{"name":"<granted-tool>","version":"<v>","args":{ … }}}
```

…and [grammar-constrained decoding](./tools.md#grammar-constrained-tool-calls)
guarantees the proposal is well-formed, while the warrant grant-check and
`inputSchema` validation remain the **only** authority (a model can never call a
tool it was not granted — SN-8).

## Serve-level controls

These are set when you start `kx serve` (operator scope, per deployment). All are
**off-digest** — they shape the live prompt only, never a committed fact or a run's
identity.

| Variable | Default | Effect |
|---|---|---|
| `KX_SERVE_AUTOGRANT` | off | Provision `kx/recipes/react-auto` + auto-grant the bundled/dialed tools so the model can choose tools turn-by-turn. |
| `KX_SERVE_REACT_TOOL_MENU` | **on** | Show the granted-tool menu so the model proposes tools autonomously. Set `0` to omit it. |
| `KX_SERVE_REACT_GRAMMAR` | **on** | Constrain tool-call decoding to the canonical envelope (llama.cpp lazy GBNF; Ollama relies on the robust parser). Set `0` to disable. |
| `KX_SERVE_REACT_SYSTEM` | _(built-in)_ | Override the curated agentic contract with a **domain persona** (e.g. an SRE copilot). |

```bash
# A persona-driven, autonomous tool-using agent server:
KX_SERVE_AUTOGRANT=1 \
KX_SERVE_REACT_SYSTEM="You are Ada, a terse SRE copilot. Prefer tools over guessing." \
  kx serve
```

## Driving an agent (one entry point, every surface)

The agent runner is the single entry point on every surface — the prompt/menu/order
machinery above is applied automatically; you just give it a goal:

```bash
# CLI
kx agent run --goal "Use your tools to compute 6 * 7, then tell me the result." --json
```

```python
# Python
import kortecx as kx
result = kx.run_agent("Use your tools to compute 6 * 7, then tell me the result.")
```

```typescript
// TypeScript
import { runAgent } from "@kortecx/sdk";
const result = await runAgent({ goal: "Use your tools to compute 6 * 7, then tell me the result." });
```

```typescript
// Chains DSL — the .agent() node is the same ReAct loop
await flow().agent("Use your tools to compute 6 * 7, then tell me the result.").run({ client });
```

> **Per-run prompts/menus.** The agentic contract and the menu are applied to *every*
> ReAct turn automatically; there is no per-call flag to set today. A per-**run**
> system-prompt override (a different persona per invocation) is a planned follow-up
> — it needs an optional recipe slot plus a durable carry across the run's turn-0
> anchor, so it ships as its own change. Per-deployment customization is available
> now via `KX_SERVE_REACT_SYSTEM` above.

## Inspecting what the agent saw

Each tool's menu rendering is previewed in the **Tools** section of the Console (the
name, description, typed inputs, and worked example the model is shown). Run
trajectories — every turn, tool call, and observation, in order — are in the
**Monitoring** section. See [Reading run outputs](./reading-run-outputs.md) and
[Observability](./observability.md).

## OSS / Cloud line

The curated agentic prompt, the granted-tool menu, grammar-constrained decoding, and
deterministic conversation ordering are all **OSS** (single-node, local models).
Hosted/closed model backends, per-tenant persona management, and a prompt/policy
marketplace are **Cloud**.
