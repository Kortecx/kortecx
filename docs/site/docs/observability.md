---
id: observability
title: Observability
sidebar_label: Observability
description: The Dashboard, gateway-wide Monitoring, per-model telemetry, failure triage, health, Prometheus metrics export, and the operator audit log.
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
liveness. The gateway also serves the standard `grpc.health.v1.Health` service for
`grpc_health_probe` / Kubernetes gRPC probes.

## Metrics export (Prometheus)

For scraping into Prometheus / Grafana / an OTLP pipeline, `kx serve` exposes a
**Prometheus text `/metrics` endpoint**, off by default and enabled with one flag:

```bash
kx serve --dev-allow-local --metrics-listen 127.0.0.1:9090
# scrape it:
curl -s http://127.0.0.1:9090/metrics
```

The metrics are **RED signals derived from the durable journal** — counters that an
operator turns into rate, error-ratio, and saturation dashboards. They are computed
on a background fold of committed facts and served from a cached snapshot, so a
scrape is fast regardless of journal size:

| Metric | Type | Meaning |
| --- | --- | --- |
| `kortecx_runs_registered_total` | counter | runs admitted (`RunRegistered` facts) |
| `kortecx_motes_committed_total` | counter | durable Mote effects (`Committed` facts) |
| `kortecx_motes_failed_total` | counter | terminal Mote failures (`Failed` facts) |
| `kortecx_motes_failed_by_reason_total{reason}` | counter | failures bucketed by reason (`timed_out`, `dead_lettered`, …) |
| `kortecx_motes_repudiated_total` | counter | committed Motes later invalidated |
| `kortecx_effects_staged_total` | counter | WORLD-MUTATING effects staged |
| `kortecx_success_ratio_basis_points` | gauge | `committed / (committed + failed)` × 10000 |
| `kortecx_journal_seq` | gauge | the highest journal sequence folded |
| `kortecx_mote_wall_p50_ms` / `kortecx_mote_wall_p95_ms` | gauge | recent-window p50/p95 execution latency (model motes) |
| `kortecx_output_tokens_window` | gauge | summed `output_tokens` over the recent window |
| `kortecx_up` / `kortecx_build_info{version}` | gauge | endpoint liveness + build |

The latency block is **honestly omitted** when no model Mote has run (e.g. an
FFI-free serve). The endpoint is **unauthenticated by design** (the scraper
convention, like the health service): bind it to loopback or a trusted network. The
canonical-projection digest is unchanged whether metrics are on, off, or scraped —
metrics only read committed facts; they are never an identity or digest input.

> OTLP push to a collector is a hardening follow-on; the Prometheus pull endpoint is
> the single-node path. Cross-party scoping + auth on the metrics surface is Cloud.

## Audit log

The long-running serve can write a **JSONL operator audit trail** — a structured,
append-only record of the run lifecycle for SIEM ingestion / accountability:

```bash
kx serve --dev-allow-local --audit-log /var/log/kortecx/audit.jsonl
```

One JSON object per line, opened in **append** mode (the trail accumulates across
restarts) and flushed on graceful shutdown:

```json
{"seq":0,"ts_ms":1718524800123,"type":"mote_dispatched","mote_id":"ab…","nd_class":"pure","kind":"pure"}
{"seq":1,"ts_ms":1718524800456,"type":"mote_committed","mote_id":"ab…","result_ref":"cd…","nd_class":"pure"}
{"seq":2,"ts_ms":1718524805000,"type":"mote_failed","mote_id":"ef…"}
```

Each line carries a monotonic `seq`, a wall-clock `ts_ms`, and **server-derived hex
ids only** — join keys back to the journal, never payload bytes, model output, or
warrant secrets. The audit log is **off the truth path**: it is best-effort, never
gates a run, and is never a digest input — the journal remains the durable truth and
the digest is recomputable from it. The operator owns retention/rotation (e.g.
`logrotate`).

**Coverage.** Every durable outcome is audited: `mote_committed` and `mote_failed`
cover **all** Motes, whether client-submitted or materialized by the live agentic
loop (shaper children, ReAct/re-plan turns). `mote_dispatched` marks **client
submissions** at admission; internally-materialized agentic children are spliced
onto the sole-writer thread and so appear as `mote_committed` / `mote_failed` without
a separate admission line (a per-child dispatch line for the agentic loop is an
additive follow-on). Filter the trail with `jq`:

```bash
jq -c 'select(.type=="mote_failed")' /var/log/kortecx/audit.jsonl
```

:::note More on the way
A failures/refusals **alerts inbox** (acknowledge / resolve), live-feed **filter
chips + JSONL export**, and a **token-economy** breakdown land with the next
observability batches. Time-travel (`kx projection --at-seq`) and run capture
(`ListCaptureRecords`) are covered in the
[Quickstart](./quickstart.md#run-your-first-blueprint) and the
[production notes in the README](https://github.com/Kortecx/kortecx/blob/main/README.md#production-notes).
:::
