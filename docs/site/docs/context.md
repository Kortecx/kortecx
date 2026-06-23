---
id: context
title: Context bundles
sidebar_label: Context
description: Named, content-addressed context bundles you attach to a run so a model reasons over your grounding — identity-bearing and exactly-once.
---

# Context bundles

PR-6 gave agents the **tools** they reason *over*; context bundles give them the
**context** they are *given*. A context bundle is a **named, content-addressed
collection** of content-store blobs (documents, notes, prior outputs) that you
attach to a run so the model reasons over your grounding.

Attaching a bundle is **identity-bearing**: a different attached context yields a
different entry `MoteId`, so a run is **exactly-once-per-(input + context)** —
the same prompt with different grounding is a different, independently-cached
run, and the same prompt with the same grounding re-derives the same identity
(idempotent re-invoke).

> **The bundle store is off the truth path.** Bundle manifests live in an
> off-journal `bundles.db` sidecar — never journaled, never a `MoteId`/digest
> input. Dropping the file loses the index (re-author to restore); it cannot move
> the canonical projection digest. The `bundle_ref` is **server-derived** (SN-8):
> you name a handle, the server derives the identity, and a bundle is visible only
> to the party that authored it (no cross-party existence oracle).

## Author a bundle

A bundle is a `namespace/collection/name` handle plus one or more items, each
referencing a blob already in the content store (see [reading run
outputs](./reading-run-outputs.md) and `kx content put`).

```bash
# Upload a file inline, then bind it under a handle:
kx context add team/ctx/spec --file design=./design.md --description "the design doc"

# Or attach refs already in the content store:
kx content put ./notes.txt              # -> ref=<hex32>
kx context add team/ctx/notes --item notes=<hex32>

kx context list
kx context get team/ctx/spec
kx context remove team/ctx/notes        # unbinds the handle; the blobs stay
```

## Edit, import & export items

The content store is **immutable** — a blob's ref is the BLAKE3 of its bytes, so a
ref never changes meaning. "Editing" an item therefore uploads the new bytes (a
**new** ref) and **re-points** the item at it, then re-derives the `bundle_ref`.
This is a pure client compose over the existing RPCs (`GetContextBundle` →
`PutContent` → `PutContextBundle`), so it touches no journal and the canonical
digest is invariant by construction. The old ref stays valid (immutable CAS), so a
run that already captured it replays unchanged — editing a bundle only affects
**future** attaches, never a run already bound to the old grounding.

```bash
# Export every item body to a directory (+ a manifest.json):
kx context get team/ctx/spec --output ./out

# Replace one item's body from a file (re-point + optional rename):
kx context edit team/ctx/spec --item design --file ./design-v2.md
kx context edit team/ctx/spec --index 0 --file ./design-v2.md --name design.md

# Drop one item (re-upsert the rest; refuses to empty the bundle):
kx context remove-item team/ctx/spec --item old-notes

# Re-set the advisory description:
kx context describe team/ctx/spec --description "the design doc, v2"
```

Select an item by its advisory `--item <name>` or by `--index <n>`; a duplicate
name is ambiguous, so pass the index.

### SDK helpers

The SDK exposes the same edit family as convenience methods over the existing
client surface. Each accepts an optional `expect_bundle_ref` / `expectBundleRef`
(the `bundle_ref` you last read): when set, a concurrent change to the bundle is
**refused** (`KxFailedPrecondition`) rather than silently overwritten — the
content-addressed `bundle_ref` is a free compare-and-swap token.

```python
body = kx.export_context_item("team/ctx/spec", "design")          # full bytes
kx.edit_context_item("team/ctx/spec", "design", b"# Design v2\n…",
                     expect_bundle_ref=bundle.bundle_ref)         # fail-closed on a race
kx.remove_context_item("team/ctx/spec", "old-notes")
```

```ts
const body = await kx.exportContextItem("team/ctx/spec", "design");
await kx.editContextItem("team/ctx/spec", "design", bytes,
                         { expectBundleRef: bundle.bundleRef });
await kx.removeContextItem("team/ctx/spec", "old-notes");
```

## Attach a bundle to a run

Pass one or more handles with `--context` (repeatable). The server resolves each
to its item refs and injects them into the entry Mote's context:

```bash
kx invoke kx/recipes/react --args '{"instruction":"summarize the design"}' \
  --context team/ctx/spec --wait
```

### Python

```python
import kortecx
kx = kortecx.KxClient("https://localhost:50151", token="…")

put = kx.put_content(open("design.md", "rb").read(), filename="design.md")
kx.put_context_bundle("team/ctx/spec", [("design", put.content_ref)],
                      description="the design doc")

run = kx.invoke("kx/recipes/react",
                {"instruction": "summarize the design"},
                context=["team/ctx/spec"], wait=True)
print(run.text)
```

### TypeScript

```ts
import { KxClient } from "@kortecx/sdk";
const kx = new KxClient("https://localhost:50151", { token });

const put = await kx.putContent(bytes, { filename: "design.md" });
await kx.putContextBundle("team/ctx/spec", [{ name: "design", contentRef: put.contentRef }],
                          { description: "the design doc" });

const run = await kx.invoke("kx/recipes/react",
                            { instruction: "summarize the design" },
                            { context: ["team/ctx/spec"], wait: true });
```

## Attach a bundle to a chain

A [Chain](./chains/dsl-reference.md) carries context **at the chain level** — the
server attaches it to the chain's entry step(s), so position in the chain is
irrelevant (there is no `context` node). Pass the handles via the `context`
option, the fluent `.context(...)`, or the CLI `--context` flag (repeatable):

```python
from kortecx.chains import chain, model

c = chain("plan > write",
          {"plan": model("kx-serve:qwen3", "Plan the doc."),
           "write": model("kx-serve:qwen3", "Write it.")},
          context=["team/ctx/spec"])           # or: .context("team/ctx/spec")
run = kx.run_chain(c, wait=True)
```

```ts
import { chain, task } from "@kortecx/sdk/chains";

const c = chain("plan > write", {
  tasks: { plan: task.model("kx-serve:qwen3", "Plan the doc."),
           write: task.model("kx-serve:qwen3", "Write it.") },
  context: ["team/ctx/spec"],                   // or: .context("team/ctx/spec")
});
await kx.runChain(c, { wait: true });
```

```bash
kx chain run "plan > write" --tasks tasks.json --context team/ctx/spec --wait
```

The handles lower **verbatim** into the request's `context_bundles` (no client-side
sort/dedup — the server owns canonicalization). The lowering is pinned
byte-identical across Python, TypeScript, and the CLI by the
[Chains golden corpus](./chains/dsl-reference.md).

## Manage bundles in the Console

The console's **Context** section (left nav → Context) is the OSS author + govern
surface:

- **Author** a bundle: a handle + a description + items added by uploading files
  (each upload is a `PutContent`) or by naming an existing content ref.
- **Review** every bundle you authored — its items (each a content ref) and the
  server-derived bundle ref.
- **View & edit** an item: expand a row to see its body in the shared viewer, then
  edit text / markdown / JSON inline (a save uploads the new bytes and re-points
  the item). Binary and media items are honestly **download-only** — no fake edit.
- **Rename** an item, **remove** one item (the bundle keeps at least one), or edit
  the bundle **description** — each a guarded re-upsert (a concurrent change is
  refused, never silently clobbered).
- **Delete** a bundle (unbinds the handle; the content-store blobs stay).

In **Chat**, the composer's attach menu has a live **Context** category: pick one
or more bundles to ground the next turn. The attached bundles show as chips above
the composer and ride that turn's run (the same identity-bearing attachment as
`--context`). The section degrades to an honest "needs a newer gateway" state when
the bundle store is absent (never a mock).

## How delivery works

At bind, the runtime resolves each attached handle to its item refs (caller-scoped,
fail-closed on an unknown handle) and folds the sorted ref-set into the **entry
Mote's identity-bearing `config_subset`**. At run time, the model executor fetches
each blob from the content store and prepends it to the prompt as a labeled
`[context <name>]` block, ahead of any upstream (parent) context. A missing ref or
a window overflow **fails closed** — the model never runs on partial or unbounded
context.

A run that attaches **no** bundle is byte-identical to one authored before context
bundles existed.

## OSS / Cloud line

Authoring, viewing, **editing / import / export**, and the deterministic
identity-bearing injection are **OSS** (all single-party, in your own scope).
Cross-party bundle sharing, a hosted bundle marketplace, and managed retention are
**Cloud** (mirrors the Connections OAuth/marketplace line — see
[Security](./security.md)).
