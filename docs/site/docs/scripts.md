---
id: scripts
title: Scripts
sidebar_label: Scripts
description: Scripting the Kortecx runtime — the --json output contract, the three shapes it takes, and exit codes.
---

# Scripts

Every read-only client verb answers `--json`, so the runtime pipes into ordinary
shell tooling. The contract has **three shapes**, and knowing which one you are
reading is the whole trick:

| Shape | Verbs | What you get |
|---|---|---|
| **One document** | the read verbs (`info`, `health`, `runs list`, `app list`, …) | exit `0`, and stdout is a single JSON value |
| **Newline-delimited** | `events` | one JSON value per line, as facts land |
| **Refusal** | a verb whose capability this build lacks | non-zero exit, and stderr names the missing build flag |

Those three are asserted verb-by-verb by
[`crates/kx-cli/tests/json_contract.rs`](https://github.com/Kortecx/kortecx/blob/main/crates/kx-cli/tests/json_contract.rs),
so this page describes a checked property rather than an intention.

## One document

```bash
kx runs list --limit 5 --json | jq -r '.runs[].instance_id'
kx info --json | jq -r '.model_id // "no model served"'
```

The keys are the wire field names — the same names the SDKs return — so a script
and an SDK client read the same shape.

## Newline-delimited

`kx events` is a tail, so it emits one JSON value per line and keeps going with
`--follow`. Read it line by line; do not buffer it and parse the whole stream as
one value:

```bash
# every terminal failure across all runs, as it happens
kx events --all --since 0 --follow --json \
  | jq -c 'select(.type == "failed")'
```

Without `--follow` it drains what exists and exits, which is the form to use in a
pipeline that must terminate.

## Refusals are honest, and worth checking

A capability that needs a build feature does not answer emptily — it refuses and
names the flag. `kx datasets list` on a gateway built without `hnsw` exits
non-zero with a message pointing at the missing feature, rather than returning
`{"datasets":[]}` (which would read as "you have none" when the truth is "this
binary cannot have any").

So branch on the exit code, not on an empty collection:

```bash
if ! out=$(kx datasets list --json 2>&1); then
    echo "dataset plane unavailable: $out" >&2
    exit 1
fi
echo "$out" | jq -r '.datasets[].name'
```

`kx health` follows the same rule the other way: it exits `0` only when the
gateway reports `SERVING`, so it works as a bare readiness probe.

## The SDKs

Both SDKs return typed objects over the same wire fields, so a script that
outgrows `jq` ports without renaming anything — see
[Chains in Python](./chains/python.md) and
[Chains in TypeScript](./chains/typescript.md) for the authoring surface, and the
[Quickstart](./quickstart.md) for installing them.

For the full command surface, `kx --help` lists every verb and `kx help <verb>`
expands one.
