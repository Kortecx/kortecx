---
id: branches
title: Branches (snapshot & edit)
sidebar_label: Branches
description: Snapshot operator-approved host files into content-addressed branches, then let the agent loop edit them in-CAS — copy-on-write, governed, and digest-invariant.
---

# Branches

A **branch** is a named, content-addressed manifest of `{path → ContentRef}`
entries over **operator-approved host files**. You *snapshot* a confined set of
host files into the content store, and the agent loop edits them **in place in
the content store (CAS)** — the host filesystem is never written in this phase.

This is the read/snapshot half of "Neon for agent workspaces": the runtime is
already a branching substrate for its own state (content-addressed CAS gives free
copy-on-write, the append-only journal gives durable lineage), and branches
extend that to *your* files.

> **Default-OFF and host-read-only.** Snapshotting reads host files only when the
> operator sets `KX_SERVE_FS_ROOT` (the same confined read root as the `fs-list` /
> `fs-read` tools). With it unset, `snapshot` returns `FAILED_PRECONDITION` and the
> runtime is byte-identical to a build with no branches. **Nothing here writes your
> host filesystem** — governed write-back is a separate, later capability.

> **The branch store is off the truth path.** Manifests live in an off-journal
> `branches.db` sidecar — never journaled, never a `MoteId`/digest input. Dropping
> the file loses the index (re-snapshot to restore); it cannot move the canonical
> projection digest. The `branch_ref` is **server-derived** (SN-8): you name a
> handle, the server derives the identity, and a branch is visible only to the
> party that authored it (no cross-party existence oracle).

## How a branch works

- **Snapshot-in** reads each confined path's bytes into the content store. Because
  the store is content-addressed, the committed ref **is** the file's content hash
  — identical bytes dedup for free.
- A **sub-branch** (`--parent`) is a point-in-time **copy-on-write fork**: it
  inherits the parent's resolved refs at create time and re-points only the paths a
  later snapshot overrides. Unchanged paths keep the parent's CAS blob (zero-copy).
  A branch is a snapshot — later edits to the parent do **not** propagate.
- **Agentic edits** stay in-CAS: the model reads a branch file's ref, proposes an
  edited body, and commits a *new* ref; the manifest advances. The host is
  untouched, so recovery and the canonical digest are unchanged.

## Snapshot files into a branch

The operator runs `kx serve` with a confined read root:

```bash
KX_SERVE_FS_ROOT=/path/to/workspace kx serve --dev-allow-local
```

Then snapshot a set of files into a branch (created on first snapshot):

```bash
kx branch snapshot team/workspace/main --path src/lib.rs --path README.md
kx branch get team/workspace/main          # the resolved {path -> ref} manifest
kx branch list                             # your branches, in handle order
```

Fork a point-in-time sub-branch and re-point one file:

```bash
kx branch create team/workspace/feature --parent team/workspace/main
kx branch snapshot team/workspace/feature --path src/lib.rs   # only lib.rs re-points
kx branch remove team/workspace/feature
```

## Edit a branch file (agentic, in-CAS)

`kx branch edit` runs the model over a branch file: it attaches the file's
**current** content as context, the model rewrites it per your instruction, the new
body commits as a fresh `ContentRef`, and the manifest **advances** to it. The host
filesystem is never written.

```bash
# the operator must serve a model; set KX_SERVE_FS_ROOT to also allow sibling reads
kx branch edit team/workspace/main --path README.md \
  --instruction "Add a one-line summary to the top; keep the rest unchanged"
kx branch get team/workspace/main          # README.md now points at the new ref
```

The edit runs through the `kx/recipes/react-edit` recipe — a single model step
(seeded only when a model is served). The model's leading `<think>` reasoning is
stripped at commit, so the committed answer **is** the new file body verbatim (no
silent transform). The model rewrites only from the attached current contents;
letting the model read *other* files in the loop (`fs-read`) is a later capability.
It never writes the host.

The edit **fails closed** if the model returns no usable file body (the branch is
left unchanged rather than advanced to an empty file). Edit quality depends on the
served model completing the rewrite; small or heavy-reasoning models may need a
retry.

Because the edit is an ordinary `Invoke`, it is **reproducible**: re-run it with a
changed instruction via `kx runs rerun --set` and only the changed sub-DAG
recomputes (the kernel's exact-equality dedup).

Power users can re-point a path to an already-committed ref directly (the low-level
step the agentic edit ends with):

```bash
kx branch advance team/workspace/main --path README.md --ref <64-hex content ref>
```

## SDKs

The Python and TypeScript SDKs expose the same surface.

```python
from kortecx import KxClient

with KxClient("http://127.0.0.1:50151", token="...") as kx:
    snap = kx.snapshot_into("team/workspace/main", ["src/lib.rs", "README.md"])
    print(snap.ingested, len(snap.items))           # files read, manifest size
    kx.create_branch("team/workspace/feature", parent="team/workspace/main")
    for b in kx.list_branches():
        print(b.handle, b.item_count, b.parent_handle)
    # agentic in-CAS edit (needs a served model)
    res = kx.edit_branch("team/workspace/main", "README.md",
                         "Add a one-line summary to the top")
    print(res.handle, res.branch_ref)        # the manifest advanced
```

```ts
import { KxClient } from "@kortecx/sdk";

const kx = new KxClient("http://127.0.0.1:50151", { token: "..." });
const snap = await kx.snapshotInto("team/workspace/main", ["src/lib.rs", "README.md"]);
console.log(snap.ingested, snap.items.length);
await kx.createBranch("team/workspace/feature", { parent: "team/workspace/main" });
const branches = await kx.listBranches();
// agentic in-CAS edit (needs a served model)
const res = await kx.editBranch("team/workspace/main", "README.md",
  "Add a one-line summary to the top");
```

## In the console

The **Branches** section (under **Data**) lists your branches, shows each
manifest's `{path → ref}` entries with a digest chip per file, and lets you create
a branch, fork a sub-branch, and snapshot a path set — when the operator has set a
read root. Without one, the section shows an honest disabled state explaining how
to enable it. Each file row has an **Edit** control that opens an instruction box
and runs the agentic in-CAS rewrite (when a model is served).

## What this is *not* (yet)

Phase A is **read + snapshot + in-CAS edit** only. Writing a branch's edited files
**back to the host filesystem** is a separate, governed capability (operator-granted
write scope + an explicit human-approval gate + full audit), sequenced after the
gateway's security hardening. The model *proposes*; the human and the runtime
*authorize* (SN-8).
