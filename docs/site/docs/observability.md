---
id: observability
title: Observability
sidebar_label: Observability
description: The Activity drawer, per-model telemetry, run cost, the alerts inbox, failure triage, health, Prometheus metrics export, and the operator audit log.
---

# Observability

Kortecx records every state change as a durable journal fact and exposes a
read-only view of that truth through the console and the SDK. Nothing here is
fabricated: each number traces to a committed fact or an honest empty state.

## Where each view lives

The console is a flat set of sections, so observability is not one "Monitoring"
destination. It is split by scope, and three of the reads are **CLI/SDK-only today**:

| What you want | Where it is |
|---|---|
| The cross-run live event tail, triage-able | the **Activity drawer** (navbar) |
| One run's metrics + time travel | the **Activity drawer**, after picking a run |
| Run history, newest-first | **Workflows → Runs** |
| One run's DAG / step detail | the run-detail view |
| Per-model telemetry + the token economy | `kx telemetry` / the SDK |
| A run's cost guardrail | `kx cost` / the SDK |
| The terminal-failure alerts inbox | `kx alerts` / the SDK |
| RED counters for Prometheus | `--metrics-listen` |

Telemetry, cost and alerts had console homes before the console flattened to its
current sections, and do not have one now. The RPCs, CLI verbs and SDK methods below
are unchanged and fully supported; only the browser surface is absent, and this page
says so rather than describing a tab you cannot open.

## The Activity drawer

**Activity** is a navbar drawer rather than a section, so the node-wide pulse is one
click from anywhere (`/activity` redirects to Workflows — the drawer replaced the
route). It opens on the **global cross-run feed** (`StreamAllEvents`), newest-first,
each row attributed to its run, with quick actions.

Pick a run — from the picker or a feed row — and it drills into that run:

- **Metrics** derived from the run's projection: mote counts by state, success rate,
  in-flight, the journal frontier, and a **commit-`seq` span** latency *proxy*. It is
  a proxy on purpose: a projection carries no wall-clock, so no millisecond figure is
  invented here. Wall-clock lives in telemetry.
- **The run-scoped event feed** and a **time-travel scrubber** — pin an `at-seq` and
  the metrics re-derive at that point, then scrub forward to live.
- **Gateway health**, always shown.

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

### How live the live feed is

Everything inside the serve that follows the journal — both event streams and the
capture, telemetry, alerts and metrics folds — **subscribes** to it. A commit wakes them
directly, so a `--follow` tail shows an event as soon as the frame can be built, and a
serve with nothing happening does not read its journal at all.

Delivery does not depend on that. Each follower tracks its own cursor and reads the
contiguous range it is owed, so notifications may coalesce without anything being missed
or repeated; the subscription decides *when* to read, never *what* was written.

```bash
# restore the previous behaviour (a 250 ms poll) — an escape hatch, not a tuning knob:
KX_SERVE_JOURNAL_WATCH=off kx serve
```

Client-side waiting (`--wait`, the SDK `wait_*` helpers) still polls: that is a separate
seam, over the wire rather than inside the serve.

### Per-mote telemetry

`ListMoteTelemetry` is the host-measured execution exhaust — wall-clock, model and
tool usage, and the committed `seq` — cursor-paged, newest-first:

```bash
kx telemetry list --limit 50
kx telemetry list --instance <hex16> --json
```

Because it is cursor-paged, any aggregate you compute from it covers **the page you
fetched, not all of history**. For an exact total, use the server-side summary below
rather than summing pages. Motes on an FFI-free serve carry no model id, so a
per-model breakdown is honestly empty there. Priced input tokens and per-expert
billing do not exist in OSS — it serves locally and has no price book to bill from.

### Token economy

`ListTelemetrySummary` totals **server-side** (a single `SUM … GROUP BY model_id`
over the same `telemetry.db` sidecar), so a long agentic run is counted exactly
rather than capped to a page:

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

### Run cost (guardrail) {#run-cost-guardrail}

`GetRunCost` reports one run's committed turns and tool calls, priced at the serve's
**micro-USD** rates against an optional **ceiling**. It is a **local budget
guardrail, not billing**: a display-only estimate over the durable counters, with an
operator-set price book (`KX_PRICING_PER_TURN_MICRO_USD` /
`KX_PRICING_PER_TOOL_CALL_MICRO_USD`; the ceiling defaults to `0` = OFF). The CLI and
both SDKs read the same RPC:

```bash
kx cost <instance-hex16>          # turns · tool calls · estimated µUSD vs. ceiling
kx cost <instance-hex16> --json   # instance_id · turns · tool_calls · estimated_micro_usd · …
```

The same estimate is available from the SDK — `client.cost.get_run_cost(instance_id)`
(Python) and `client.cost.getRunCost(instanceId)` (TypeScript).

The readout is **honest** at every state: a **zero-baseline** price book (rates
unset) shows the counts with **no fabricated dollar figure**; a run over its ceiling is
flagged; a serve without the cost admin degrades to a not-wired note. Per-token /
per-expert **billing** — priced input tokens, invoices, credits — is a
[Cloud](https://github.com/Kortecx/kortecx#cloud) capability, never invented in OSS.

## Failure triage

A failed event row surfaces the journal's `FailureReason` as a short, scannable
**reason badge** (e.g. `TIMED OUT`, `VALIDATOR REJECTED`, `WORKER CRASHED`,
`DEAD-LETTERED`) next to the `FAILED` pill, mirroring the closed enum in the
runtime, and it is **filterable** via the live-feed free-text filter. A row that
carried no reason shows no badge — the reason is never invented.

## Alerts inbox

`ListAlerts` is a read-only operator inbox of **terminal failures** — the runtime's
durable signal that a run gave up. It is folded from the
journal's terminal `Failed` facts (dead-letters and worker-reported terminal
failures); the liveness retries (`TIMED OUT` / `WORKER CRASHED`, which re-dispatch)
are deliberately excluded, so a row here means a run that is genuinely done and
failed.

```bash
# the inbox, newest-first and paginated:
kx alerts list
kx alerts list --instance <hex16> --limit 50
kx alerts list --json   # alert_id · mote_id · reason_class · reason_code · severity · seq
```

Each alert carries the failed mote plus the journal `seq` of the
`Failed` fact, so it joins straight back to the run. The inbox lives in a
rebuildable, off-journal `alerts.db` sidecar that
is **folded from committed facts**: delete it and restart, and the same alert set
re-materializes — it is derived, never authoritative, and never changes the canonical
projection digest. When the serve has no sidecar (an older build) the view degrades
honestly to "not wired".

**Scope note.** Admission **refusals** (a `SubmitRun` rejected up front with a
`kx-refusal-code`) are *not* in this inbox — they are synchronous responses that never
become journal facts, and they surface in the live feed instead.

The **triage lifecycle** (acknowledge / resolve), an **alert-rule engine**, and
outbound **notifications** (Slack / PagerDuty / webhook) are a managed
[Cloud](https://github.com/Kortecx/kortecx#cloud) capability — OSS ships the durable
read-only view and invents nothing beyond it.

## Health

Gateway liveness is inferred from a cheap unary round-trip on an interval (the same
probe the connect flow uses) and rendered as a `LIVE` / `DEGRADED` / `DOWN` pill in
the Activity drawer. From the CLI, `kx health` reports the same liveness and **exits
`0` only when the gateway answers `SERVING`**, so it works as a bare readiness probe.
The gateway also serves the standard `grpc.health.v1.Health` service for
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
