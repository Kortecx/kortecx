---
id: blueprint-builder
title: Blueprint builder
sidebar_label: Blueprint builder
description: Author a workflow visually — a vetted PURE/MODEL palette, chained agents, edge instructions, and recipe search — across the console, the SDKs, and the CLI.
---

# Blueprint builder

A **blueprint** is a reusable workflow: a small DAG of steps the runtime compiles,
admits, and runs. You can author one **visually** in the console, **fluently** in
the SDKs (the `BlueprintBuilder`), or **declaratively** with the
[Chains DSL](./chains/dsl-reference.md). Every path produces the same thing — a
`SubmitWorkflow` request the **server** compiles and runs.

## The trust model (why the client never sends warrants)

The builder sends only the **topology + params**. The server assigns each step's
logic from its kind, derives all identity, and builds **every per-step warrant from
your grants** — the client cannot inject an executable body or a warrant. A tampered
client DAG only changes what is *proposed*, never the authority it is granted. The
palette is therefore deliberately small: **PURE** (a deterministic transform),
**MODEL** (an agent step routed to the served model), and **TOOL** (fire one
registered tool — the server resolves it + builds the per-step warrant). A MODEL
step can also be tagged with tools in the Chains DSL (`model@tool`) to become a
[deterministic-agentic step](./chains/dsl-reference.md#the-deterministic-agentic-step--modeltool-pr-9b)
— authorable across the SDK/CLI today; its bounded-loop execution + a visual editor
land in PR-9b-2.

## The visual builder

Open **Blueprints → New blueprint** (`/blueprints/new`):

- **+ Agent** adds a MODEL step; **+ Pure step** adds a PURE step; **+ Tool** adds a
  TOOL step (pick a registered tool + its JSON args).
- **Drag** to arrange; **drag handle-to-handle** to connect two steps with a data
  edge (the parent's result flows into the child).
- **Click a node** to open its config drawer — name, model, prompt (Monaco), the
  opt-in [reasoning mode](./agent-runner.md#reasoning-mode), and typed JSON params.
- **Click an edge** to attach an **instruction file** (see below).
- **Build & run** compiles the DAG on the server and routes you to the live run.

The builder runs a client-side **acyclicity precheck** (a workflow DAG must be
acyclic) so an invalid graph is caught before submit; the server's compiler remains
the authority.

### Orchestration patterns

Beyond single nodes, the toolbar scaffolds whole **multi-agent patterns** — the same
topologies the SDK `supervisor()` / `consensus()` methods and the `kx swarm` verb author:

- **+ Swarm** — N parallel agents → a gather.
- **+ Supervisor** — a planner → workers (in parallel) → an integrator.
- **+ Consensus · judge** — N voters → a model judge that selects the best.
- **+ Consensus · majority** — N voters → an exact-equality majority vote (server-reduced).

Each button drops a **pre-wired cluster of ordinary agent / pure nodes** onto the canvas —
no new node kind — which you then fill in per node (pick a model, edit the prompt) and run. The
pattern is just a scaffold of the nodes you already know; it lowers to the same DAG the SDK and
CLI produce. See [Orchestration patterns](./patterns.md) for the full family.

### Chained agents

Wire two MODEL steps with a data edge and you have a **chain of agents** — the first
agent's output becomes the second agent's context. The same shape is one line in the
Chains DSL:

```python
from kortecx.chains import model, chain

# draft, then critique — a 2-agent chain
c = chain("draft > critique", {
    "draft":    model("kx-serve:qwen3-4b-q4_k_m", "Draft an answer to the task."),
    "critique": model("kx-serve:qwen3-4b-q4_k_m", "Critique and improve the draft."),
})
run = await client.run_chain(c, wait=True)
```

### Edge instructions

An edge can carry an **instruction file** — context passed *between* two steps. In
the builder, click the edge and write the instruction; at run time it is **prepended
to the downstream agent's prompt** so the instruction genuinely reaches the agent.
(Durable, content-addressed context bundles are a later batch; in Tier-1 the
instruction rides in the child's prompt.)

### Clone to edit

Any committed run can be **reconstructed into the builder** — open a run and choose
**Build from this** (`/blueprints/new?clone=<instanceId>`). The builder reads the
run's DAG (topology + each step's kind / model / prompt / params) and opens it for
editing. The submit is always a **new** workflow with **new identity by
construction** — the original run is never touched.

### Replay vs re-run

From a run you can:

- **Run again** — re-invoke the *same* blueprint with the *same* args. This is
  **idempotent**: the same recipe + args resolve to the already-committed result (the
  memoizer), so "running again" honestly **joins** the existing run — same digest.
- **Clone** — open the blueprint with the prior inputs **prefilled** to tweak and run
  as a new use case (new identity).

## The SDK builder

```typescript
import { BlueprintBuilder } from "@kortecx/sdk/web";

const b = new BlueprintBuilder(0);
const draft = b.addStep({ kind: "model", modelId: "kx-serve:m", prompt: "Draft it." });
const critique = b.addStep({ kind: "model", modelId: "kx-serve:m", prompt: "Improve it." });
b.addEdge({ parent: draft, child: critique, edge: "data" });
const handle = await client.submitWorkflow(b.build());
```

```python
from kortecx.blueprints import BlueprintBuilder, StepInput, EdgeInput

b = BlueprintBuilder(seed=0)
draft = b.add_step(StepInput(kind="model", model_id="kx-serve:m", prompt="Draft it."))
critique = b.add_step(StepInput(kind="model", model_id="kx-serve:m", prompt="Improve it."))
b.add_edge(EdgeInput(parent=draft, child=critique, edge="data"))
handle = client.submit_workflow(b.build())
```

## Portable blueprints — export & import

A lowered chain can be saved as a **portable blueprint JSON** — the exact shape
`kx blueprint run --file` consumes. Save it, version it in git, share it, and re-run
it on any serve. (Cross-party sharing / a marketplace is a Cloud capability.)

**Export from the CLI** — `--emit-blueprint` writes the blueprint alongside the run;
pair with `--dry-run` to export offline (no gateway):

```bash
kx chain run "research > review" \
  --task research='{"kind":"model","prompt":"Research the topic."}' \
  --task review='{"kind":"model","prompt":"Critique the findings."}' \
  --emit-blueprint plan.json --dry-run        # lower + validate + write, no submit

kx blueprint run --file plan.json --wait      # run it (here, or anywhere)
kx blueprint import --file plan.json          # validate + summarize offline (no run)
```

**Export from the SDKs** — `to_blueprint()` returns the dict; `export(path)` writes JSON:

```python
import kortecx as kx
flow = kx.flow().agent("Research the topic").then("Critique the findings")
flow.export("plan.json")                       # portable artifact
req = kx.Chain.from_blueprint_file("plan.json")  # → a SubmitWorkflowRequest
client.submit_workflow(req, wait=True)
```

```typescript
import { flow, Chain } from "@kortecx/sdk";
await flow().agent("Research the topic").then("Critique the findings").export("plan.json");
const req = await Chain.fromBlueprintFile("plan.json");   // → a SubmitWorkflowRequest
await client.submitWorkflow(req, { wait: true });
```

The artifact is **self-describing** (each step's `kind` is explicit) and **portable**:
`model_id` is left as authored — empty binds the **serve's** model at submit (SN-8), so
the same blueprint runs against whatever model a target serve hosts unless you pin a
specific `model_id`. Export → import re-compiles to a **byte-identical**
`SubmitWorkflowRequest` (and, on a fixed model, the identical committed result). The CLI
emits the args-separated form and the SDKs the params-folded form; both import
equivalently.

## Finding a blueprint

The console's Blueprints section has a **search box** backed by `SearchRecipes` —
type an intent and the gateway ranks its provisioned recipes by match. The same is
available from the CLI and SDKs:

```bash
kx recipe list                       # the catalog + advisory metadata
kx recipe search "agent loop" --limit 5
```

```python
hits = client.search_recipes("agent loop", limit=5)
for h in hits:
    print(h.score_bp, h.recipe.handle, h.recipe.tags)
```

The score is **display-only** (integer basis points; `10000` = an exact handle). A
search *surfaces* a recipe — it never *invokes* one. `kx invoke` (and the form's
**Run**) stay the authorization gate.
