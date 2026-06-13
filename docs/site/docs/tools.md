---
id: tools
title: Tools
sidebar_label: Tools
description: MCP tool discovery and the ReAct tool loop in Kortecx.
---

# Tools

Kortecx agents call real **MCP tools** inside the live ReAct loop — every tool
turn committed as a durable fact. Discovery is advisory (`kx tools list` /
`kx tools score`); authorization is always the runtime's, never a score (see
[Security](./security.md#model-proposes-runtime-enforces)).

:::note Coming soon
Full tool documentation — registering tools, the idempotency contract, and the
`kx/recipes/react` loop — lands with a later docs PR. For now, see the
[Quickstart agent loop](./quickstart.md#run-the-agent-loop) and the
[`tools` CLI reference in the README](https://github.com/Kortecx/kortecx/blob/main/README.md#client-commands).
:::
