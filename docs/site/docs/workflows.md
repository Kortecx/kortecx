---
id: workflows
title: Workflows
sidebar_label: Workflows
description: Your runs as bordered cards with clean names and a per-card action menu — open, re-run, clone, build-from, rename, and export (record or full results); plus the Blueprints catalog.
---

# Workflows

The **Workflows** section is your run history. Every time you run a blueprint —
from Chat, the Blueprints catalog, or the visual builder — the run is enumerated
here: durable runs recovered from the journal (`ListRuns`) merged with this
browser's session records. Each run is a **bordered card** with a clean display
name and a per-card action menu.

> A run is an immutable, committed fact. Workflows is presentation over that
> truth: renames and session history are **client-local** (per gateway endpoint,
> in your browser); the runs themselves live in the journal and never change.

## The card

- **Name.** The headline is a clean, humanized name (`kx/recipes/echo` →
  *Echo*). A local **rename** wins over it; the raw handle stays as a secondary
  mono chip so the exact recipe is never lost. A durable run recovered from the
  journal carries a **journal** badge.
- **Open.** Click the name to open the run's detail — the live **DAG**, the
  **table**, **artifacts**, and the **activity / time-travel** tabs.
- **Filter.** Filter the grid by name, id, or blueprint handle.
- **Clear local history.** Clears only *this browser's* session records — the
  durable journal runs (and your local renames) stay. The label is honest about
  what it removes.

## The action menu

The **⋯** menu on each card carries the run's actions:

- **Open in new tab** — the run detail in a new browser tab (`rel="noopener"`).
- **Run again** — re-invoke the same blueprint + args. This is **idempotent**:
  the same recipe + args resolves to the same already-committed result, so
  "running again" honestly *joins* the committed run rather than duplicating it.
- **Clone** — open the run's blueprint with its inputs **prefilled**; tweak and
  run it as a new use case.
- **Build from this** — reconstruct the run's graph in the [visual
  builder](./blueprint-builder.md) to add agents, wire steps, and run a new
  workflow.
- **Rename** — set a client-local display name (this browser only).
- **Export** — download the run **record** as JSON (id, name, blueprint handle,
  args, timestamp).
- **Export with results** — download a richer bundle that also fetches the
  committed **DAG** and each step's **resolved output text** over
  `GetProjection` / `GetContent`. Available once the run has a terminal step.
- **Share** / **Schedule** — shown as disabled **Cloud** chips. Sharing across
  parties and scheduling recurring runs are managed-cloud capabilities; the
  console never fakes a control it cannot honor.

## The Blueprints catalog

The [**Blueprints**](./blueprint-builder.md) section is the companion catalog —
the templates you run. It mirrors the same card language:

- Each blueprint is a card with a clean name, its **description**, advisory
  **tags**, and a **version** chip; the raw handle stays a secondary mono chip.
- **Search** the catalog by intent (`SearchRecipes`, display-only ranking — a hit
  *surfaces* a blueprint, never runs it).
- Clicking a card opens its **input form** in a slide-over drawer; fill the
  server-described fields and run it (the run lands back here in Workflows).
- The card menu offers **Run**, **Open in new tab**, **View contract** (the
  free-param contract in a read-only editor), **Edit in builder** (clone-to-edit
  in the visual builder), **Rename**, **Export** (the blueprint definition), and
  the same honest **Cloud** Share / Schedule chips.
- **+ New blueprint** opens the visual builder.

## See also

- [Reading run outputs](./reading-run-outputs.md) — how a committed run resolves
  to readable text across the console (and what an exported bundle contains).
- [Blueprint builder](./blueprint-builder.md) — author and edit the blueprints
  Workflows runs.
- [Chat](./chat.md) — the conversational front door; each message is a run that
  appears here.
