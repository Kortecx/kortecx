---
id: apps
title: Apps
sidebar_label: Apps
description: Author, save, and run a durable, reusable App — a kortecx.app/v1 envelope over a portable blueprint.
---

# Apps

An **App** is the durable, reusable unit of work you name, save, list, and re-run.
It is a `kortecx.app/v1` **envelope** that *wraps* the existing portable
[blueprint](./blueprint-builder.md) with:

- **references** — by-*reference* pointers to context items, tools, connections,
  datasets, plus a minimal **prompt / rule / skill / memory** artifact rail. A
  reference is a name + a content ref (or a registry id); it never inlines bytes.
- a **steering config** — four axes (model + routing, tools + grants, context +
  data, guards + budgets) the server re-resolves at bind.
- per-step **replay** intent.

An App carries **no authority**. There is no warrant, grant, secret, credential, or
`instance_id` in the envelope — when you run an App the server re-compiles its
blueprint and re-resolves *every* warrant from your own grants (SN-8). Saving and
running an App can never widen what you are allowed to do.

The catalog is **caller-scoped** and lives in an off-journal `apps.db` sidecar; the
server derives the App's identity (`app_ref`) from the canonical envelope. Apps are
local to one runtime in this release — cross-instance import (sharing) is a Cloud
capability.

## Author in Python

```python
import kortecx as kx

app = (
    kx.app("research-assistant")
    .blueprint(kx.flow().agent("Research the topic.", tools=["mcp-echo/echo"]))
    .rule("no-pii", body="Never reveal personal data.")
    .steer(max_turns=8, max_tool_calls=6)
    .describe("A grounded research agent")
)

app.save()                       # persist to the catalog (uploads pending bodies first)
app.run({"topic": "kortecx"})    # compile the blueprint + run it (exactly-once)
```

The reference primitives are thin by design (extensible later): `prompt` / `rule` /
`memory` are named text artifacts stored in the content store; a `Skill` is a named
(instructions + tool wish set) bundle. Pass a body (`body=...`, uploaded at `save`)
or a content ref (`ref=...`) you already uploaded.

## Author in TypeScript

```ts
import { app, flow } from "@kortecx/sdk";

const a = app("research-assistant")
  .blueprint(flow().agent("Research the topic.", { tools: ["mcp-echo/echo"] }))
  .rule("no-pii", { body: "Never reveal personal data." })
  .steer({ maxTurns: 8, maxToolCalls: 6 });

await a.save();                       // Node zero-config client, or pass { client }
await a.run({ topic: "kortecx" });
```

The browser entrypoint (`@kortecx/sdk/web`) is explicit-client by design — pass a
`client` to `save` / `run`.

## The CLI

```sh
# Author an envelope OFFLINE from a blueprint file (no gateway):
kx app new "Echo Demo" --from-blueprint echo.dag.json \
  --max-turns 8 --max-tool-calls 6 --tag demo --output echo.app.json

kx app save echo.app.json            # persist (handle defaults apps/local/echo-demo)
kx app list                          # browse the catalog
kx app get apps/local/echo-demo      # show the summary (--output writes the envelope)
kx app run apps/local/echo-demo --wait   # compile the blueprint + run it
kx app export apps/local/echo-demo --output echo.app.json   # the round-trip artifact

# POC-5a — agentically scaffold the App's project tree into its CoW branch:
kx app scaffold apps/local/echo-demo --goal "Echo the user's input" --wait
kx app files apps/local/echo-demo            # list the scaffolded files
kx app cat apps/local/echo-demo README.md    # print one file's body

# POC-5b — lock the App (agentic in-CAS edits are then refused):
kx app lock apps/local/echo-demo
kx app unlock apps/local/echo-demo
```

`kx app run` is "the runtime as a function": it fetches the saved App's blueprint and
submits it; the server warrants every step from your grants.

## The envelope format

The envelope is canonical JSON — sorted keys, compact, integers only — so it
serializes byte-identically across the Rust CLI, the Python SDK, and the TypeScript
SDK (pinned by `tests/golden/apps/`). The `kx app export` / `to_envelope` form is
pretty-printed but round-trips to the same canonical bytes. The `schema` field
(`"kortecx.app/v1"`) is the version gate — a reader fails closed on an unknown
schema. `media_type` is carried per context reference at the envelope layer (the
bind-time codec drops it).

The optional `branch_handle` field names the App's per-App project branch. By
convention an App's project branch shares the App's own handle (one App, one
branch), so `kx app files <handle>` and the console resolve it directly.

## Scaffold a project tree (POC-5a)

An App is more than an envelope — it has a **project**: a small tree of files the
agent authors and you can edit in place. `kx app scaffold` (or the console's **New
App** button) drives a server-side agentic loop that writes a **fixed skeleton** into
the App's content-addressed (CoW) branch — the model authors the *content* of each
file, the structure is fixed and testable:

```
README.md            prompts/system.md     skills/main.md
app.json             rules/guardrails.md
```

The scaffold runs in the background and is observed from **real** signals — the
branch manifest growing + a status phase (`planning → writing → done`) — never a
cosmetic timer. It is durable and resumable: a re-`scaffold` writes only the files
still missing. Edits stay **in-CAS** — the host filesystem is never written.

## The single-App IDE (POC-5d)

**Open** an App (the console **Open** button, or `kx app files` / `kx app cat`) into a
full-screen **IDE** with three tabs:

- **Files** — the project tree + a Monaco editor over the App's CoW branch. Edit a file
  two ways:
  - **directly** — type the new contents in Monaco and **Save** (`PutContent` →
    `AdvanceBranch`; the host is never written). The CLI equivalent is
    `kx app edit <handle> <path> --from <file>`.
  - **agentically, with a review gate** — describe the change; the model rewrites the
    file and you **review the diff** (current vs proposed) before it commits. **Approve**
    advances the manifest; **Reject** discards (nothing is written). This is the same
    `react-edit` loop as `kx branch edit`, split so the change is previewed first.
- **Lineage** — the App's blueprint rendered as an **editable graph** (reorder / add /
  remove / configure steps + edges). **Save to App** persists a new App version
  (`SaveApp`); only the blueprint is replaced — every other rail (references, steering,
  replay, inputs) is carried verbatim. A blueprint the visual editor can't faithfully
  round-trip (e.g. an `exec` step) opens read-only. Dump the structure with
  `kx app structure <handle>`.
- **Chat** — chat with the App in context.

The active tab and selected file are URL-addressable (`?tab=`/`?path=`), so refresh and
deep links are stable. See [Branches](./branches.md) for the CoW mechanics.

## Run an App

**Run** an App from the IDE header or the **Workflows** catalog. If the App declares an
`input_schema`, a run drawer collects the inputs (they fold into the entry model step);
otherwise it runs in one click. The run routes to its live DAG. OSS runs **one App at a
time** — multi-app chaining and scheduling are Cloud capabilities. The CLI equivalent is
`kx app run <handle>` (`--arg k=v` per input).

## Lock an App (POC-5b)

`kx app lock <handle>` (or the **Security › Policies** section) **fully freezes** an
App: a locked App refuses BOTH an in-CAS **file** edit (`AdvanceBranch`) AND a
**structure** save from the lineage editor (`SaveApp`) at the write chokepoints
(`FAILED_PRECONDITION`, refusal code `LOCKED_BRANCH`). `kx app unlock` re-enables
edits. Locking is a per-party policy decision (off the truth path); losing it fails
OPEN (editing is restored, never bricked). The console pre-disables the write controls
on a locked App, but the runtime is the authoritative gate.

## The Apps console

Open **Apps** in the sidebar. Browse your saved Apps, **Inspect** the full envelope,
**Run** one (it routes to the live run), click **New App** to scaffold a fresh App, or
**Open** an App into the file tree + editor. **Share** is a Cloud capability (shown
honest-disabled). Per-App locks live in the **Policies** section
([policies.md](./policies.md)).

## Chains node

There is **no `app()` Chains-DSL node**: a Chains node is a *step* in a DAG, while an
App is a *whole-run artifact* that wraps a complete blueprint. An App sits one level
above a chain — `app().blueprint(flow()...)` — it consumes a chain, it is never a
node inside one (the same reasoning as the agent-runner).
