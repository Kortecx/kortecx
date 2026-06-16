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
  first, each row attributed to its run. The feed is **triage-able**: toggle kind
  chips (with per-kind count badges), filter by run id / mote / reason free-text,
  and **export the filtered buffer as NDJSON**.
- **Telemetry** — the host-measured execution exhaust (`ListMoteTelemetry`):
  wall-clock, model/tool usage, and the committed `seq`, cursor-paged.

### Live-feed triage

The live feed is a structured log, so it is filtered like one — entirely
client-side over the buffered tail, the server stream is untouched:

- **Kind chips** toggle which event kinds show (`committed`, `failed`,
  `repudiated`, `effect_staged`, `run_registered`); each carries a count badge
  over the current buffer.
- A **free-text filter** narrows by run id, mote hex, or the human reason label.
- **Export** writes the *filtered* rows as **NDJSON** — one server-derived object
  per line (hex join keys only, never payloads), the same shape the CLI emits.

From the CLI, the same global tail is filterable by kind and exports as NDJSON:

```bash
# the global cross-run tail, filtered to failures + commits (one JSON object per line):
kx events --all --kind committed,failed --json > feed.ndjson
# a live, filtered follow:
kx events --all --kind failed --follow
```

### Per-model telemetry rollup

The Telemetry tab derives a **per-model rollup** client-side over the loaded
window — `count`, `p50` / `p95` wall-clock ms (nearest-rank), and total
`output_tokens` per model — beside a KPI strip of the window aggregates. The table
is captioned **"over the last N motes (this page, not all-time)"** and is honestly
**absent when no model mote ran** (e.g. an FFI-free serve, where motes carry no
model id). Cost and per-expert billing are shown as a disabled **Cloud** tile: OSS
serves locally and has no price, input-token, or expert entity to bill.

### Token economy

Beside the page-windowed rollup, the Telemetry tab shows an **all-runs token
economy** — output tokens and wall-clock per model, summed **server-side** so a
long agentic run is totalled exactly (not capped to a page). It is backed by
`ListTelemetrySummary` (a single `SUM … GROUP BY model_id` over the same
`telemetry.db` sidecar):

```bash
kx telemetry summary               # per-model output tokens + wall-clock, all runs
kx telemetry summary --instance <hex16>   # scoped to one run
kx telemetry summary --json        # model_id · count · total_output_tokens · total_wall_clock_ms
```

```python
summary = client.list_telemetry_summary()         # Python SDK
for row in summary.rows:
    print(row.model_id, row.total_output_tokens)
```

The economy is **token-only and honest**: there is no fabricated "tokens saved"
delta — no durable counterfactual baseline exists, so none is invented (a run's
reasoning mode is recoverable per-mote from its definition, but no aggregate
savings number is computable). **Cost / $** stays the disabled
[Cloud](https://github.com/Kortecx/kortecx#cloud) tile.

## Failure triage

A failed event row surfaces the journal's `FailureReason` as a short, scannable
**reason badge** (e.g. `TIMED OUT`, `VALIDATOR REJECTED`, `WORKER CRASHED`,
`DEAD-LETTERED`) next to the `FAILED` pill, mirroring the closed enum in the
runtime, and it is **filterable** via the live-feed free-text filter. A row that
carried no reason shows no badge — the reason is never invented.

## Alerts inbox

The **Alerts** sub-tab in Monitoring is a read-only operator inbox of **terminal
failures** — the runtime's durable signal that a run gave up. It is folded from the
journal's terminal `Failed` facts (dead-letters and worker-reported terminal
failures); the liveness retries (`TIMED OUT` / `WORKER CRASHED`, which re-dispatch)
are deliberately excluded, so a row here means a run that is genuinely done and
failed.

```bash
# the same inbox from the CLI (newest-first, paginated):
kx alerts list
kx alerts list --instance <hex16> --limit 50
kx alerts list --json   # alert_id · mote_id · reason_class · reason_code · severity · seq
```

Each alert deep-links to the failed run's graph and carries the journal `seq` of the
`Failed` fact. The inbox lives in a rebuildable, off-journal `alerts.db` sidecar that
is **folded from committed facts**: delete it and restart, and the same alert set
re-materializes — it is derived, never authoritative, and never changes the canonical
projection digest. When the serve has no sidecar (an older build) the view degrades
honestly to "not wired".

**Scope note.** Admission **refusals** (a `SubmitRun` rejected up front with a
`kx-refusal-code`) are *not* in this inbox — they are synchronous responses that never
become journal facts, and they surface in the live feed instead.

The **triage lifecycle** (acknowledge / resolve), an **alert-rule engine**, and
outbound **notifications** (Slack / PagerDuty / webhook) are a managed
[Cloud](https://github.com/Kortecx/kortecx#cloud) capability and appear here as an
honest-disabled card — OSS ships the durable read-only view.

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

:::note See also
Time-travel (`kx projection --at-seq`) and run capture (`ListCaptureRecords`) are
covered in the [Quickstart](./quickstart.md#run-your-first-blueprint) and the
[production notes in the README](https://github.com/Kortecx/kortecx/blob/main/README.md#production-notes).
OTLP export is on the roadmap; today metrics are Prometheus text and traces are the
durable journal itself.
:::
