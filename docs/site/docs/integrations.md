---
id: integrations
title: Integrations — connect Slack & Notion, run a governed App
sidebar_label: Integrations
description: Connect a bundled Slack or Notion connector by credential name, author an App that uses it, fire it on a schedule, and govern its irreversible actions from the console's approvals inbox — the end-to-end "integration comes alive" chain.
---

# Integrations

An **integration** is an external provider (Slack, Notion, Gmail, Discord, …) a `kx serve`
runtime can act on — read a channel, post a message, search a workspace — through the
**external MCP gateway**. kortecx ships a set of **bundled connectors** so the common
providers work out of the box, and the whole flow stays governed: the credential is
referenced by **name** (never inlined, D81/SN-8), an App declares exactly which tools it
may fire, and every irreversible action can be **held for an operator's approval** (D114).

This page walks the full chain end to end:

1. **Connect** a curated integration (and set its secret).
2. **Author** an App that uses it.
3. **Automate** it on a trigger.
4. **Govern** its actions from the console.

Each deeper capability has its own page — [Tools & Connections](./tools.md),
[Authoring a connector](./authoring-a-connector.md), [Managing secrets](./managing-secrets.md),
[Apps](./apps.md), [Authoring a trigger](./authoring-a-trigger.md). This is the narrative
that threads them.

## 1. Connect a curated integration

A **curated** connector is a bundled sidecar dialed by a single `--provider` flag — the
runtime fills in the connector command and the credential-ref name for you.

```bash
# Register the dial (this also discovers the connector's tools):
kx connections add --provider slack     # → server "slack",  reads KX_SLACK_CREDENTIAL
kx connections add --provider notion    # → server "notion", reads KX_NOTION_CREDENTIAL

# Set the secret VALUE by name (write-only; never echoed back):
kx secrets set KX_SLACK_CREDENTIAL  '{"bot_token":"xoxb-…"}'
kx secrets set KX_NOTION_CREDENTIAL '{"token":"secret_…"}'
```

The bundled connectors today are **Slack** (`slack/{post_message,read_channel,search,list_channels}`),
**Notion** (`notion/{search,read_page,create_page,append_block}`), **Gmail**, and **Discord**.

:::tip Out-of-the-box dial
`--provider slack` stores a **bare** connector name (`kx-connector-slack`). At dial time the
runtime first looks for that binary **beside the running `kx`** (its install sibling), then
falls back to your `PATH`. So a co-installed bundled connector dials with **no manual PATH
setup**. A connector you built yourself is still reachable by an absolute path or on `PATH`.
If a `--provider` dial reports the connector is unreachable, the sidecar binary is simply not
installed next to `kx` or on your `PATH` yet. Run `kx connections doctor` (a local, offline
advisory — no server needed) to see, per provider, whether its bundled connector resolves as a
`kx`-sibling or on `PATH`, and how to install it if not:

```bash
kx connections doctor                 # check every curated provider
kx connections doctor --provider slack --json
```
:::

You can dial any of the four tools directly to check the wiring before you build an App:

```bash
kx connections fire --name slack --tool list_channels --args '{}'
```

## 2. Author an App that uses it

Declare the integration on the App with a curated builder (`.with_slack()`); grant the agent
step the tools it may fire, namespaced by the **connection name** you registered above
(`slack/…`); and `secrets([...])` scopes the run's warrant to that credential so — and only so
— the App may dial it. The App is **saved** and runs through **RunApp**, which re-resolves the
connection + secret scope at bind (D177). The SDK spine is identical across Python and
TypeScript (golden parity).

```python title="Python"
import kortecx as kx

app = (
    kx.app("launch-digest")
      .blueprint(
          kx.flow().agent(
              "Summarise the last 20 messages in #launch and post a one-line digest.",
              # tools are namespaced by the connection NAME registered above ("slack"):
              tools=["slack/read_channel", "slack/post_message"],
          )
      )
      .with_slack()                       # declares the kx-connector-slack connection
      .secrets(["KX_SLACK_CREDENTIAL"])   # scopes the run to this credential (SN-8)
)
app.run()                                 # → RunApp (connection + secret_scope resolved)
```

```typescript title="TypeScript"
import { app, flow } from "@kortecx/sdk";

const launchDigest = app("launch-digest")
  .blueprint(
    flow().agent(
      "Summarise the last 20 messages in #launch and post a one-line digest.",
      { tools: ["slack/read_channel", "slack/post_message"] },
    ),
  )
  .withSlack()
  .secrets(["KX_SLACK_CREDENTIAL"]);

await launchDigest.run();
```

Use `.with_notion()` / `.withNotion()` (scoping `KX_NOTION_CREDENTIAL`, tools namespaced
`notion/…`) the same way. An App may name more than one integration.

:::note Tool namespace
An agent grants tools by `<connection-name>/<tool>` — the name you gave the connection at
`kx connections add` (the curated `--provider slack` names it **`slack`**). `.with_slack()`
declares WHICH connector the App dials; the agent's `tools=[…]` list is what it is allowed to
fire. Keep the two in step — grant `slack/…` for a `--provider slack` connection.
:::

:::tip Fires on your local models — both engines
The agentic loop reliably fires connector tools on the local OSS models kortecx positions on —
**both** `kx serve` engines: llama.cpp (Gemma) and Ollama (`gemma3:12b`). Each engine emits
tool-calls in its own format, so the runtime constrains the model to a parseable shape: llama.cpp
arms a lazy grammar; Ollama applies a non-strict `{"tool_call":…} | {"answer":…}` response
schema (the model still answers freely, it just can't emit a malformed call). This is on by
default; `KX_SERVE_OLLAMA_TOOL_UNION=0` reverts to unconstrained free-form decoding on Ollama.
:::

## 3. Automate it on a trigger

A **trigger** fires a saved, credentialed App unattended — on a schedule, a webhook, or a
gRPC poke — through the same durable path. Because posting to a channel is **irreversible**,
gate it with `--require-approval` so the runtime **stages and withholds** each post until you
say so (see [Authoring a trigger → Firing an App on a schedule](./authoring-a-trigger.md#firing-an-app-on-a-schedule)):

```bash
# 09:00 on weekdays, New York time; hold irreversible posts for approval:
kx triggers add launch-digest \
  --app launch-digest \
  --cron "0 9 * * 1-5" \
  --timezone America/New_York \
  --require-approval
```

## 4. Govern it from the console

When a `require-approval` App reaches an irreversible action, the runtime **pauses** it and
records the request. Govern it from **Monitoring → Approvals** in the console (or the CLI):

- The **Approvals** inbox lists each withheld action — its tool, intent, run, deadline, and
  the run's **spend so far** (turns · tool calls · estimated µUSD). A count badge on the
  **Monitoring** nav item shows how many are awaiting you.
- **Grant** releases the staged action to fire exactly once; **Deny** rejects it and the
  chain dead-letters. The decision **survives a crash** — a granted action is never re-asked.

```bash
kx approvals list                 # what is awaiting a decision
kx approvals grant <REQUEST_ID>   # release the staged action (fires once)
kx approvals deny  <REQUEST_ID>   # reject it (the chain dead-letters)
kx cost <RUN_INSTANCE_ID>         # the run's local spend estimate
```

Read-only diagnosis (read a channel, search a workspace) auto-proceeds; only world-mutating
actions (a post, a page create) hit the gate. The whole loop — connect, author, schedule,
approve — is **durable and replayable**: every fired action is journaled and auditable.

## OSS / Cloud line

Everything here runs on a **single-node** `kx serve` with **local** OSS models. The bundled
connectors, the local approvals inbox, and the local scheduler are OSS. A hosted trigger
gateway at scale, a multi-tenant credential marketplace, and cross-app orchestration are
**Cloud** (D129 / GR19).
