---
id: reading-run-outputs
title: Reading run outputs
sidebar_label: Reading run outputs
description: How committed results are stored content-addressed and resolved back to text in the console, the SDKs, and the CLI.
---

# Reading run outputs

Every Mote that commits produces a **result** — the bytes it computed. Kortecx
stores those bytes **content-addressed** and hands you back a small pointer. This
page explains how the output is stored, how to read it back, and how the console
turns a pointer into the text you actually see.

## How a result is stored

When a Mote commits, its output bytes are written **once** to the gateway's
content store, keyed by their own hash:

```
result_ref = blake3(result_bytes)      // a 32-byte content address
```

The durable journal records only that 32-byte `result_ref` — never the payload.
The commit protocol verifies the bytes are present in the content store
**before** it writes the `Committed` fact (so a ref always resolves), and because
the ref *is* the hash of the content, the lookup can never go stale: identical
output always lands at the same address, and the same address always returns the
same bytes.

This is why a run's graph stays small and fast to query no matter how large its
outputs are — the topology holds pointers, the content store holds bytes.

## Reading it back

Resolve a ref with **`GetContent`** (one ref) or **`GetContentBatch`** (up to 64
refs in a single round trip — the N+1 collapse for tables and feeds). Both are
**run-scoped**: a ref is readable only by the run that produced it (pass the
run's `instance_id`), and an unauthorized, missing, or malformed ref comes back
as a **uniform empty item** — there is no existence oracle.

### TypeScript

```ts
import { KxClient } from "@kortecx/sdk";

const kx = new KxClient("http://127.0.0.1:50151");
const run = await kx.invoke("kx/recipes/echo", { topic: "hello" }, { wait: true });

// One result:
const bytes = await kx.getContent(run.resultRef, run.instanceId);
console.log(new TextDecoder().decode(bytes));

// Many at once (preview-sized), e.g. every committed Mote in a run:
const items = await kx.getContentBatch(refs, {
  instanceId: run.instanceId,
  maxBytesPerItem: 4096n,
});
for (const it of items) {
  if (it.missing) continue; // uniform denial — not an error
  console.log(it.contentRef, new TextDecoder().decode(it.payload), it.truncated);
}
```

### Python

```python
from kortecx import KxClient

kx = KxClient("http://127.0.0.1:50151")
run = kx.invoke("kx/recipes/echo", {"topic": "hello"}, wait=True)

# One result:
data = kx.get_content(run.result_ref, run.instance_id)
print(data.decode())

# Many at once (preview-sized):
items = kx.get_content_batch(refs, instance_id=run.instance_id, max_bytes_per_item=4096)
for it in items:
    if it.missing:        # uniform denial — not an error
        continue
    print(it.content_ref, it.payload.decode(errors="replace"), it.truncated)
```

### CLI

```sh
# kx invoke --wait inlines a printable result as result_utf8 (text) + result_hex.
kx invoke kx/recipes/echo --args '{"topic":"hello"}' --wait --json

# Or fetch any committed ref directly (run-scoped with --instance):
kx content get --ref <result_ref_hex> --instance <instance_id_hex>
kx content get --ref <result_ref_hex> --out result.bin   # large/binary → a file
```

## How the console shows it

The web console never makes you read a hash. Wherever a result appears — the run
**table**, the **DAG** node, the **Artifacts** list, the **Activity** feed, the
node **inspector**, and **chat** answers — the **resolved text is the headline**
and the content address rides alongside as a small **digest chip** you can click
to copy. The console batches all the refs visible on a surface into a single
`GetContentBatch`, so a wide table or a long feed resolves in one round trip, and
each row repaints from `resolving…` to its text as the batch lands.

Honest states are kept honest: an uncommitted Mote shows a dash (no result yet),
an empty result reads `(empty)`, non-UTF-8 output reads `binary · N B` (never a
fake text headline), and a preview truncated at the per-item clamp links to the
full artifact. None of these are ever shown as a bare hash.

:::note Display only
Resolved content is for reading. Identity and authority always come from the
runtime: the ref is a content address (it changes when the bytes change), never a
capability — reading it still goes through the run-scoped, uniform-denial check.
:::
