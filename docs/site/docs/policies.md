---
id: policies
title: Policies
sidebar_label: Policies
---

# Policies — the per-App agent-write gate (POC-5b)

An App's project lives in a content-addressed (CoW) branch, and the agent can rewrite
its files in place (see [Apps](./apps.md)). **Policies** is where you control that
authority: a per-App **lock** that decides whether the agent may modify an App.

## Lock / unlock

A locked App refuses **every** agentic in-CAS edit at the single write chokepoint —
the `AdvanceBranch` step that re-points a file to its rewritten body. A refused edit
returns `FAILED_PRECONDITION` with the structured refusal code `LOCKED_BRANCH` (clients
act on the code, never the prose), and the console disables the per-file **Edit**
affordance with an honest notice. Unlocking restores edits.

```sh
kx app lock apps/local/my-app      # agentic edits now refused
kx app unlock apps/local/my-app    # edits re-enabled
```

In the console, open **Policies** in the sidebar: each App shows its lock state and a
**Lock** / **Unlock** control.

## Guarantees

- **Caller-scoped.** You can only lock / unlock / observe your OWN Apps — there is no
  cross-party oracle.
- **Off the truth path.** A lock is a per-party *policy* decision, not journalled
  state: it never moves the canonical projection digest, and it is rebuildable.
- **Fails OPEN.** A lock is an *availability* gate, not an integrity gate — if the lock
  store is lost it recreates empty (every App reads unlocked). Losing the file can
  never brick an App's editing; it only drops the (re-settable) policy.

## What is Cloud

Richer policy (run-mode warrant editors, role-based access control across parties,
agent-access protocols) is a Cloud capability. OSS Policies is the single-party
per-App lock + modify gate.
