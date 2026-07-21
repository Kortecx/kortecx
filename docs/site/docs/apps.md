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
- a **steering config** — four axes (model + routing, tools + grants + `reach`,
  context + data, guards + budgets) the server re-resolves at bind (see
  [Permissions & the capability manifest](#permissions--the-capability-manifest)).
- per-step **replay** intent.

An App carries **no authority**. There is no warrant, grant, secret, credential, or
`instance_id` in the envelope — when you run an App the server re-compiles its
blueprint and re-resolves *every* warrant from your own grants (SN-8). Saving and
running an App can never widen what you are allowed to do.

The catalog is **caller-scoped** and lives in an off-journal `apps.db` sidecar; the
server derives the App's identity (`app_ref`) from the canonical envelope. An App is
**portable**: you can export it (with its content closure) to a `.kxapp` bundle,
import that bundle under your own account, or clone an App locally — all fail-closed
and single-runtime. (Signed provenance across untrusted parties is a Cloud capability.)

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
kx app manifest apps/local/echo-demo # capability manifest — needs vs. what you have
kx app run apps/local/echo-demo --wait   # compile the blueprint + run it
kx app export apps/local/echo-demo --output echo.app.json   # the by-ref envelope

# Portable Apps — export a self-contained bundle, import it, or clone locally:
kx app export apps/local/echo-demo --bundle echo.kxapp   # envelope + content closure
kx app export apps/local/echo-demo --bundle echo.kxapp --with-data   # + RAG payloads
kx app import echo.kxapp                    # reconcile under YOUR account (fail-closed)
kx app clone apps/local/echo-demo my-copy   # a local frozen copy (records lineage)

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

## Portable Apps — bundle, import & clone

An App references its content (prompts, rules, context, skill instructions, RAG data)
**by content-store ref**, so the plain envelope isn't self-contained. A
**`kortecx.appbundle/v1` bundle** packages the canonical envelope **plus its
transitive content closure** into a single portable `.kxapp` file:

- `kx app export <handle> --bundle <file>` writes the bundle. It walks the envelope's
  content refs, fetches each blob at full size, and names the bundle by the App's
  handle-free `app_digest`. `--with-data` also inlines RAG dataset payloads (they can be
  large, so they are opt-in). The same bundle format is emitted byte-identically by the
  Python (`App.export(bundle=…)`) and TypeScript (`App.export(path, { bundle: true })`)
  SDKs — pinned by `tests/golden/apps/bundle_corpus.json`.
- `kx app import <bundle>` reconciles a bundle **fail-closed under your own account**:
  it re-validates the envelope, verifies the declared `app_digest`, shows the carried
  instruction bodies for review (pass `--yes` to skip the prompt non-interactively),
  uploads each blob (the server re-derives an identical content ref and dedups), then
  saves the App under a new local handle with a `source_digest` lineage stamp.
- `kx app clone <handle> <newname>` makes a **local frozen copy** under a new name
  (content is already resident, so nothing transfers) and records the source lineage.

**Connections and secrets never travel in a bundle.** An App carries only a credential
**name** and a userinfo-free connector descriptor; the importer re-registers the
connection by name (`kx connections add`). Until then the App fails closed at run with
`missing integration`. The envelope itself can carry no warrant, grant, secret, or
credential value — the server re-validates it on import, so importing an App can never
widen what you are allowed to do. `source_digest` is a **lineage hint, not a signature**
(unauthenticated provenance in this release).

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
App** button) drives a server-side agentic loop that writes that tree into the App's
content-addressed (CoW) branch.

Every scheduled App gets the same **base set**, whose *content* the model authors for
your goal:

```
README.md            prompts/system.md     skills/main.md
app.json             rules/guardrails.md
```

On top of that, the model **plans additional files for your specific goal** — another
skill per distinct capability, a separate rule for a policy worth stating on its own,
reference material the agent consults at run time. The base five are preserved (the
planner may only add, never replace them), so a scaffolded project is always a superset
of the list above. With no served model the scaffold degrades to the base set alone.

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

`kx app run` prefers the server-side **`RunApp`** RPC (below); on an older gateway it
falls back to the legacy client-orchestrated `GetApp` → `SubmitWorkflow` (which does not
honor the App's connection/secret references).

## Integrations — an App that USES a connection (G2)

An App can carry a *pointer* to an [integration connection](./tools.md) and dial it
inside its agentic loop. Declare the connection (a bare **credential-ref name**, never a
secret) with `.with_connection(...)` / `.with_gmail()`; the credential is added to the
App's `guards.secret_scope`:

```python
import kortecx as kx

app = (kx.app("gmail-agent")
       .blueprint(kx.flow().agent("Search my unread Gmail and summarise it.",
                                  tools=["gmail/search"]))
       .with_gmail()                      # declare the connection + scope KX_GMAIL_CREDENTIAL
       .steer(max_turns=4, max_tool_calls=2))
app.save(handle="apps/local/gmail-agent")

# server-side RunApp — honors the pointer (`run_app` is a client method).
kx.default_client().run_app("apps/local/gmail-agent", wait=True)
```

```typescript
import { app } from "@kortecx/sdk";

await app("gmail-agent")
  .blueprint(flow().agent("Search my unread Gmail and summarise it.", { tools: ["gmail/search"] }))
  .withGmail()
  .steer({ maxTurns: 4, maxToolCalls: 2 })
  .save({ handle: "apps/local/gmail-agent" });

await client.runApp("apps/local/gmail-agent", { wait: true });
```

At run time **`RunApp`** reads the *validated stored envelope* server-side (the client
cannot forge references — SN-8), resolves each `references.connections` entry against
**your own** registered connection by name, and sets the run warrant's
`SecretScope::AllowList` to the App's `guards.secret_scope`. That is what lets the agent
dial a *credentialed* connector (e.g. Gmail): the broker precheck requires the tool's
credential to be in the warrant's secret scope. Register the connection first with
`kx connections add --provider gmail` (see [Tools & connections](./tools.md)); a
referenced-but-unregistered connection fails fast with `missing integration: <name>`.
Because the pointer is a bare *name*, a shared App resolves **each operator's own**
credentials — the OSS single-instance sharing model (multi-party on one instance is
Cloud). `guards.secret_scope` may only name a credential one of the App's referenced
connections provides (least-privilege).

## Grounded Apps — context, rules & datasets (RAG-on-App)

An App carries a **declarative knowledge rail** — the instructions, rules, and corpora it
reasons over — and `RunApp` wires that rail into the run server-side, so the App
**self-grounds** instead of needing a hand-authored blueprint. Every entry is by-reference
(a bare name or a content ref, never inline authority):

- **`.rule(name, body=…)` / `.prompt(...)` / `.memory(...)` / `.context(name, ref=…)`** —
  text the agent must follow. Each becomes a labeled context item on the entry step
  (`rule:<name>`, `prompt:<name>`, …), fail-closed if the body is missing from the content
  store.
- **`.dataset(dataset_ref)` (alias `.rag(...)`)** — a corpus to ground over. At run the
  entry step is granted the read-only **`retrieve`** tool and steered to search
  `dataset_ref` live in the loop (exactly how [`react-rag`](./agentic-rag.md) grounds).
  **Ingest the corpus first** with `kx datasets ingest <dataset_ref> …` (the
  *reference-existing* model); a named dataset absent from the server fails fast with
  `app grounds on dataset "…" but no such dataset is ingested`.

```python
import kortecx as kx

# 1) ingest the corpus once (operator step)
#    $ kx datasets ingest research --file corpus.md

# 2) author an App that grounds on it
app = (kx.app("analyst")
       .blueprint(kx.flow().agent("Answer the question, grounded in the corpus."))
       .dataset("research")                       # references.datasets → retrieve@1 at run
       .rule("cite", body="Always cite the retrieved passages."))
app.save(handle="apps/local/analyst")

# 3) run — the agent retrieves from `research` and follows the rule
kx.default_client().run_app("apps/local/analyst", {"q": "…"}, wait=True)
```

```typescript
import { app, flow } from "@kortecx/sdk";

await app("analyst")
  .blueprint(flow().agent("Answer the question, grounded in the corpus."))
  .dataset("research")
  .rule("cite", { body: "Always cite the retrieved passages." })
  .save({ handle: "apps/local/analyst" });

await client.runApp("apps/local/analyst", { wait: true });
```

At run **`RunApp`** resolves the rail off the *validated stored envelope* (SN-8): the
context/rule/prompt/memory artifacts merge into the entry step's identity-bearing context,
and each declared dataset folds a `retrieve@1` grant onto the entry step — a grant the
operator authorizes by having ingested the dataset (not a caller escalation: `retrieve`
only reads operator corpora, with no egress/filesystem/secret reach). You can also declare
datasets and a tool wish through the steering config
(`steering_config.context.dataset_refs`, `steering_config.tools.requested_grants`); tool
wishes are intersected server-side (`wish ∩ your grants ∩ fireable`) — a wish never becomes
authority. On a gateway built without the `hnsw` retrieval seam a declared dataset
honest-degrades to an *ungrounded* run rather than erroring. In the **Apps** console, the
**New App** form exposes a "Ground on dataset" chip + a guidance-rule field over these same
rails.

### Self-contained grounding (a corpus that travels)

A dataset reference may **carry its own corpus**: `references.datasets[].cas_refs` names the
content-store blobs the dataset spans, and `kx app export --bundle --with-data` ships those
blobs inside the bundle. On the importing side the corpus **materializes itself on first
run** — so a shared App grounds on the bytes it carries, with *none* of the author's
datasets present. Nothing to pre-ingest, no `kx datasets ingest` chore.

Details worth knowing:

- **It is automatic.** The first `RunApp` ingests the carried corpus; later runs reuse the
  index. There is no import flag, and any App already in your catalog gains this.
- **The physical dataset is scoped**, named `<declared>.app-<hash>` — keyed on the corpus
  bytes and on the server's live embed model. This keeps a carried corpus from silently
  merging into a same-named local dataset of yours. It is *collision avoidance, not an
  access boundary*: a hosted OSS server is single-tenant, and any caller can already read
  any dataset. Swapping the embed model re-derives the name, so the App re-ingests rather
  than querying an index built in a different embedding space.
- **Text only.** The corpus is embedded server-side, which needs UTF-8 text; a non-text
  blob is skipped.
- **It degrades, never breaks.** Export *without* `--with-data` still records `cas_refs`
  but ships no blobs; such an App falls back to grounding on a pre-ingested dataset of the
  declared name — the behavior described above — and `kx app import` tells you which
  datasets still need one.

## Permissions & the capability manifest

An App declares a **request** for capability (the tools, connections, and model it wants
to use); the runtime grants only the **intersection** with what *you* — the party running
it — are actually allowed to do. A wish is never authority.

**See what an App needs vs. what you have** with `kx app manifest <handle>` (or the
**View details** panel in the Apps console). It diffs the App's requested tools /
connections / model against your live policy — your fireable tools, your registered
connections, the models this instance serves — and marks each capability *satisfied*,
*MISSING*, or *inherited*. It is read-only and gates nothing; a run resolves the same
intersection server-side.

```bash
kx app manifest apps/local/research-assistant
#   model: (served default)  [served]
#   tools (reach: explicit):
#     retrieve@1 [satisfied]
#     gmail/search@1 [MISSING — not granted or not fireable]
#   connections:
#     mcp+stdio://gmail [MISSING — register with `kx connections add`]
```

**Tool reach.** The tool axis carries a `reach` selector (`steering_config.tools.reach`):

- `explicit` (the default) grants exactly the tools the App enumerates in
  `requested_grants`, intersected with your policy.
- `inherit_principal` grants the App the *whole set of tools you are allowed to fire* —
  a convenience for a personal App that should adapt to whatever you have, **bounded by
  your policy** (never more than you can fire yourself). Set it with `steer(reach=…)` in
  the SDK.

Either way the server computes `materialised = request ∩ your-policy` — a strict
intersection that only ever narrows.

**Model route.** If an App names a model (`steer(model=…)` →
`steering_config.model.model_route`), a run routes to that model **only if this instance
serves it** — otherwise the run **fails closed** with a clear error rather than silently
running on a different model. Clear the route to use the served default. The manifest
flags an unserved route before you run.

## Lock an App (POC-5b)

`kx app lock <handle>` (or the **Security › Policies** section) **fully freezes** an
App: a locked App refuses BOTH an in-CAS **file** edit (`AdvanceBranch`) AND a
**structure** save from the lineage editor (`SaveApp`) at the write chokepoints
(`FAILED_PRECONDITION`, refusal code `LOCKED_BRANCH`). `kx app unlock` re-enables
edits. Locking is a per-party policy decision (off the truth path); losing it fails
OPEN (editing is restored, never bricked). The console pre-disables the write controls
on a locked App, but the runtime is the authoritative gate.

## The Apps console

Open **Apps** in the sidebar. Browse your saved Apps and **Run** one (it routes to the
live run). Each card's overflow (⋯) menu holds **View details** (the summary + the
[capability manifest](#permissions--the-capability-manifest) — needs vs. what you have),
**Open project**, **Inspect** the envelope, **Download** a portable `.kxapp` bundle, and
**Duplicate** (clone locally). **Import** a bundle from the section header; click **New App** to
scaffold a fresh App. **Share** across parties is a Cloud capability (shown
honest-disabled). Per-App locks live in the **Policies** section
([policies.md](./policies.md)).

## Chains node

There is **no `app()` Chains-DSL node**: a Chains node is a *step* in a DAG, while an
App is a *whole-run artifact* that wraps a complete blueprint. An App sits one level
above a chain — `app().blueprint(flow()...)` — it consumes a chain, it is never a
node inside one (the same reasoning as the agent-runner).
