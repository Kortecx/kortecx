---
id: observability
title: Observability
sidebar_label: Observability
description: The Dashboard, gateway-wide Monitoring, per-model telemetry, failure triage, and health.
---

# Observability

Kortecx records every state change as a durable journal fact and exposes a
read-only view of that truth through the console and the SDK. Nothing here is
fabricated: each number traces to a committed fact or an honest empty state.

## The Dashboard

The **Dashboard** (a Workspace nav item) is the operator's at-a-glance landing. It
folds data already on the wire into a small, honest KPI grid plus a live activity
tail:

- **Runs** — the durable run count (`ListRuns` merged with the per-endpoint session
  history).
- **Output tokens** and **p50 wall ms** — summed / percentiled over the **loaded
  telemetry window** (`ListMoteTelemetry`). The sublabel ("over last N motes") is
  literal: telemetry is cursor-paged, so these cover the page you have loaded, not
  all of history.
- **Serving models** — the number of models backing the live serve loop
  (`ListModels`). On an FFI-free serve this is honestly `0` / `—`.
- **Recent runs** + **Live activity** — the newest runs (click through to a run's
  detail) and the cross-run event tail.

The default landing is still **Chat** — the Dashboard is an additional entry point,
not a redirect.

## Monitoring

The **Monitoring** section is the deeper, gateway-wide view, with three
URL-addressable tabs:

- **Overview** — cross-run rollups: run counts by blueprint, the self-correction
  trails (`ListReplanRounds` / `ListReactTurns`), the action-capture stream
  (`ListCaptureRecords`), and gateway health. Each panel degrades to an honest
  "not wired on this gateway" note rather than a hollow placeholder.
- **Live feed** — the continuous cross-run event tail (`StreamAllEvents`), newest
  first, each row attributed to its run.
- **Telemetry** — the host-measured execution exhaust (`ListMoteTelemetry`):
  wall-clock, model/tool usage, and the committed `seq`, cursor-paged.

### Per-model telemetry rollup

The Telemetry tab derives a **per-model rollup** client-side over the loaded
window — `count`, `p50` / `p95` wall-clock ms (nearest-rank), and total
`output_tokens` per model — beside a KPI strip of the window aggregates. The table
is captioned **"over the last N motes (this page, not all-time)"** and is honestly
**absent when no model mote ran** (e.g. an FFI-free serve, where motes carry no
model id). Cost and per-expert billing are shown as a disabled **Cloud** tile: OSS
serves locally and has no price, input-token, or expert entity to bill.

## Failure triage

A failed event row surfaces the journal's `FailureReason` as a short label
(e.g. `TIMED OUT`, `VALIDATOR REJECTED`, `WORKER CRASHED`, `DEAD-LETTERED`) next to
the `FAILED` pill, mirroring the closed enum in the runtime. A row that carried no
reason shows no label — the reason is never invented.

## Health

Gateway liveness is inferred from a cheap unary round-trip on an interval (the same
probe the connect flow uses) and rendered as a `LIVE` / `DEGRADED` / `DOWN` pill on
the Dashboard and in Monitoring. From the CLI, `kx health` reports the same
liveness.

:::note More on the way
Structured log filtering, failure-reason humanization across the live feed, an
alerts inbox, and JSONL export land with a later observability batch. Time-travel
(`kx projection --at-seq`) and run capture (`ListCaptureRecords`) are covered in the
[Quickstart](./quickstart.md#run-your-first-blueprint) and the
[production notes in the README](https://github.com/Kortecx/kortecx/blob/main/README.md#production-notes).
:::
