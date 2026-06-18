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

Authoring, viewing, and the deterministic identity-bearing injection are **OSS**.
Cross-party bundle sharing, a hosted bundle marketplace, and managed retention are
**Cloud** (mirrors the Connections OAuth/marketplace line — see
[Security](./security.md)).
