# Monitoring & Analytics — Feature Documentation

## Overview

Kortecx provides enterprise-grade observability across all platform operations: workflow runs, expert performance, agent metrics, system health, and activity logs — all persisted to NeonDB with real-time WebSocket updates. Data flows from the [Quorum Engine](QUORUM_ENGINE.md) through WebSocket events and REST APIs into the frontend for visualization and storage.

---

## Data Flow

```
Engine (Python)
  │  Orchestrator executes workflow
  │  ├─ Broadcasts WS events (agent.spawned, thinking, step.complete, etc.)
  │  ├─ Logs to quorum_operations (asyncpg fire-and-forget)
  │  ├─ Saves artifacts to disk (steps/execution/...)
  │  └─ POSTs logs to frontend /api/logs
  │
  ▼
Frontend (Next.js)
  │  useWorkflowWS receives events, updates UI
  │  ├─ Persists step_executions to NeonDB on completion/failure
  │  ├─ Auto-captures metric snapshots (stale > 1 min)
  │  └─ Analytics queries aggregate from all tables
  │
  ▼
NeonDB (PostgreSQL)
  ├─ metrics (time-series snapshots)
  ├─ logs (structured activity logs)
  ├─ step_executions (per-step execution metrics)
  ├─ execution_audit (full audit trail)
  ├─ quorum_operations (agent-level operations)
  └─ quorum_metrics (scheduler metrics)
```

---

## Dashboard (`/dashboard`)

Real-time operations center with live engine integration.

### Top-Level Stats

| Metric | Source | Description |
|--------|--------|-------------|
| Active Agents | Engine live metrics | Currently running agents (green pulsing "Live" badge) |
| Tasks Today | NeonDB | Workflow runs in the last 24 hours |
| Tokens Used | NeonDB | Total inference tokens consumed |
| Avg Latency | NeonDB | Mean response time across all runs |

### Panels

| Panel | Content |
|-------|---------|
| **Task Queue** | Running and queued tasks with progress bars |
| **Workflow Runs** | Recent completions and failures with status badges |
| **Provider Health** | Status of all configured AI providers (local and cloud) |
| **Expert Pool** | Active and idle agent counts |

Live engine data is preferred when available. When the engine is unreachable, the dashboard falls back to the most recent database snapshots.

---

## Analytics (`/analytics`)

Enterprise performance dashboard with aggregated metrics.

### Weekly Metrics

| Metric | Description |
|--------|-------------|
| Tasks Completed | Total workflow runs completed this week |
| Tokens Consumed | Aggregate token usage |
| Estimated Cost | Cost calculation based on token rates |
| Success Rate | Percentage of runs completing without errors |

### Daily Task Chart

Bar visualization showing task volume per day, with color coding for completed (green), failed (red), and in-progress (amber).

### Provider Usage

Token consumption breakdown per inference provider, showing relative usage of Ollama, llama.cpp, and cloud providers.

### Expert Performance Table

| Column | Description |
|--------|-------------|
| Expert | Name and role |
| Total Runs | Number of workflow steps executed |
| Success Rate | Percentage of successful completions |
| Avg Latency | Mean execution time |
| Cost per Run | Average token cost |

### Workflow Performance (New)

Five-stat summary grid sourced from the `step_executions` table:

| Stat | Description |
|------|-------------|
| Total Runs | All workflow executions |
| Success Rate | Completed / total |
| Avg Duration | Mean workflow execution time |
| Total Tokens | Aggregate tokens across all runs |
| Est. Cost | Computed from token usage |

### System Health (New)

Four gauge visualizations sourced from live engine metrics:

| Gauge | Range | Source |
|-------|-------|--------|
| Active Agents | 0 - max concurrent | Engine `/api/metrics/live` |
| Error Rate | 0% - 100% | Computed from recent runs |
| Avg Latency | 0ms - max | From step_executions |
| Uptime | 0% - 100% | Engine health check |

### Activity Feed (New)

Scrollable list of the 20 most recent log entries with:

- Level-colored indicators (green=info, amber=warning, red=error, purple=critical)
- Timestamp and source service
- Clickable entries linking to full log view in `/monitoring/logs`

---

## Monitoring (`/monitoring`)

### Logs (`/monitoring/logs`)

Real-time log viewer with terminal-style UI:

| Feature | Description |
|---------|-------------|
| Level Filtering | Toggle debug, info, warning, error, critical |
| Source Search | Filter by source service |
| Message Search | Full-text search across log messages |
| Time Range | 5m, 30m, 1h, 6h, all |
| Auto-scroll | Pin to bottom for live tailing |
| Download | Export as JSONL file |
| Expandable Metadata | Click any log entry to see full JSON metadata |

### Log Entry Format

```json
{
  "id": "log-abc123",
  "level": "info",
  "source": "orchestrator",
  "message": "Workflow 'research-pipeline' completed in 8.2s",
  "metadata": {
    "workflowId": "wf-001",
    "runId": "run-abc123",
    "totalTokens": 14500,
    "durationMs": 8200
  },
  "timestamp": "2026-03-21T12:00:08Z"
}
```

### Alerts (`/monitoring/alerts`)

| Feature | Description |
|---------|-------------|
| Severity Levels | info, warning, error, critical |
| Count Badges | Per-severity count in filter tabs |
| Acknowledge | Mark an alert as seen |
| Resolve | Mark an alert as resolved |
| Filtering | Filter by severity, time range, source |

---

## Database Tables

### `metrics`

Time-series metric snapshots, auto-captured when the latest snapshot is stale (>1 minute).

| Column | Type | Description |
|--------|------|-------------|
| id | UUID | Primary key |
| active_agents | INTEGER | Currently running agents |
| tasks_today | INTEGER | Runs in last 24h |
| tokens_used | INTEGER | Aggregate tokens |
| avg_latency_ms | FLOAT | Mean response time |
| cpu_percent | FLOAT | System CPU |
| memory_mb | FLOAT | System memory |
| gpu_percent | FLOAT | GPU utilization |
| created_at | TIMESTAMPTZ | Snapshot time |

### `logs`

Structured activity logs from all platform services.

| Column | Type | Description |
|--------|------|-------------|
| id | UUID | Primary key |
| level | TEXT | debug / info / warning / error / critical |
| source | TEXT | Originating service |
| message | TEXT | Log message |
| metadata | JSONB | Additional context |
| created_at | TIMESTAMPTZ | Log time |

### `execution_audit`

Full audit trail for every workflow execution.

| Column | Type | Description |
|--------|------|-------------|
| id | UUID | Primary key |
| run_id | TEXT | Workflow run identifier |
| step_id | TEXT | Step identifier |
| event_type | TEXT | Event name |
| payload | JSONB | Full event payload |
| created_at | TIMESTAMPTZ | Event time |

### `model_comparisons`

Side-by-side model comparison results from re-run experiments.

| Column | Type | Description |
|--------|------|-------------|
| id | UUID | Primary key |
| workflow_id | TEXT | Workflow identifier |
| model_a | TEXT | First model |
| model_b | TEXT | Second model |
| results | JSONB | Comparison metrics |
| created_at | TIMESTAMPTZ | Comparison time |

---

## Metrics Auto-Capture

The `/api/metrics` endpoint implements automatic snapshot persistence:

1. On every GET request, check the timestamp of the latest snapshot
2. If the latest snapshot is older than 1 minute, capture a new one
3. Merge live engine data (if available) with database aggregates
4. Persist the new snapshot to the `metrics` table

This ensures continuous metric history without requiring external cron jobs or scheduled tasks. The dashboard's regular polling naturally drives snapshot creation.

---

## Frontend Hooks

| Hook | Polling | Source | Purpose |
|------|---------|--------|---------|
| `useLiveMetrics` | 5s | Engine `/api/metrics/live` | Real-time system metrics |
| `useStepExecutions` | SWR | NeonDB `step_executions` | Step execution history |
| `useExpertStats` | SWR | NeonDB aggregation | Expert performance stats |
| `useMetricsHistory` | SWR | NeonDB `metrics` | Historical metric snapshots |

---

## API Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/api/metrics` | Current metrics (with auto-capture) |
| GET | `/api/metrics/live` | Live engine metrics (proxied) |
| GET | `/api/logs` | Query logs with filtering |
| POST | `/api/logs` | Create log entries (from engine) |
| GET | `/api/monitoring/alerts` | List alerts |
| PATCH | `/api/monitoring/alerts` | Acknowledge or resolve alerts |
| GET | `/api/workflows/executions` | Step execution history |
| GET | `/api/analytics` | Aggregated analytics data |

---

## Enterprise Metrics Tracked

| Category | Metrics |
|----------|---------|
| Throughput | Total runs, runs/day, tokens/sec |
| Latency | Avg response time, p95 latency, queue wait time |
| Reliability | Success rate, error count, failure rate |
| Resources | CPU%, GPU%, memory MB, active agents |
| Cost | Tokens used, cost per run, daily spend |
| Quality | Expert success rate, model comparison scores |

---

## Related Documentation

- [Quorum Engine](QUORUM_ENGINE.md) — Source of execution events and metrics
- [Workflow Builder](WORKFLOW_BUILDER.md) — Execution artifacts and step metrics
- [Expert System](EXPERT_SYSTEM.md) — Expert performance tracking
- [MCP Servers](MCP_SERVERS.md) — Integration monitoring
