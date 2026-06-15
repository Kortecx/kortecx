---
id: rerun-with-changes
title: Re-run with changes
sidebar_label: Re-run with changes
description: Fork a prior run with edited inputs and re-invoke — only the changed sub-DAG recomputes, an unchanged re-run returns the existing result.
---

# Re-run with changes

Take a run you already executed, **edit one or more of its inputs, and run it
again** — getting fast, cheap iteration because only the part of the graph the
change actually affects recomputes. Unchanged inputs return the existing result
with no recompute and no re-fired side-effects.

This is **not replay.** In kortecx, *replay* means deterministic reproduction of
an existing run (recovery). **Re-run with changes** is a **new** run with edited
parameters. The two are orthogonal — replay stays intact.

## Why it's cheap (a property of the kernel)

A Mote's identity is content-addressed: it folds the recipe step, its config, and
its parents' identities. So when you change one input:

- the **changed** Mote and everything **downstream** of it get new identities and
  recompute;
- **sibling** and **upstream** Motes keep the same identity and are served from
  the exact-equality cache (zero work);
- if you change **nothing**, every Mote matches and the run **dedups** to the
  existing result — no new facts, and any side-effects are **not** re-fired.

You get Nix/Bazel-style memoized re-execution for free; "Re-run with changes"
just adds the plumbing to **capture, edit, and resubmit** the inputs.

## How inputs are captured

When you invoke a recipe, the gateway captures the submitted args into a small,
off-journal sidecar keyed by the run's `instance_id`. The capture is:

- **off the truth path** — the args never become committed facts and never enter
  any Mote's identity, so they cannot affect the run's results or the projection
  digest;
- **rebuildable to empty** — deleting it loses only the re-run pre-fill
  convenience; a run still serves and replays normally;
- **audit-scoped** — the caller is recorded as the server-resolved principal
  (audit only, never a read filter); single-tenant on the open-source node, with
  cross-tenant isolation enforced by the cloud auth wall — the same boundary as
  feedback and uploads.

A re-run is just a new **`Invoke`** with edited args — the same admission path as
any other invocation. It never uses a client-supplied warrant.

## In the console

Open a run from the **Runs** list, then **Re-run with changes**. The recipe form
opens pre-filled with the run's original inputs (recovered durably even in a fresh
browser session). Edit any field and submit:

- if your edits match the originals, the console shows **"Showing existing result
  — nothing changed"** and links you to the existing result (honest dedup);
- if a step in the prior run was **world-mutating**, you get a
  confirm-before-fire prompt (its side-effects will run again);
- otherwise the new run opens and only the changed steps recompute.

## In the CLI

```sh
# Re-run a prior run, overriding one or more inputs (repeatable --set).
kx runs rerun <instance-hex16> --set topic="new topic" --set count=5 --wait
```

A `--set key=value` whose value parses as JSON keeps its type (`--set count=5` →
the number `5`, `--set on=true` → the boolean); otherwise it is a string. The
gateway still validates every arg against the recipe form. An unchanged re-run
returns the existing result; a gateway that didn't capture the inputs (an older
run, or an older serve) degrades honestly — fall back to `kx invoke`.

## In the SDKs

Fetch the captured inputs, edit, and invoke again.

### TypeScript

```ts
import { KxClient } from "@kortecx/sdk";

const kx = new KxClient({ endpoint: "http://localhost:50051" });
const { handle, args } = await kx.getRunInputs(instanceId);
const run = await kx.invoke(handle, { ...args, topic: "new topic" });
```

### Python

```python
from kortecx import KxClient

kx = KxClient(endpoint="http://localhost:50051")
inputs = kx.get_run_inputs(instance_id)
run = kx.invoke(inputs.handle, {**inputs.args, "topic": "new topic"})
```

`getRunInputs` / `get_run_inputs` raises **not found** for a run with nothing
captured and **unimplemented** for a gateway without the sidecar — handle both by
falling back to a blank form or a direct `invoke`.

## Notes

- The serve journal holds one run; re-running keeps the same `instance_id` and
  produces a new answer (a new terminal Mote) for changed inputs.
- See also [Reading run outputs](./reading-run-outputs.md) for how a run's
  results are stored and resolved.
