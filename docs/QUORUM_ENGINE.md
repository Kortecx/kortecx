# Quorum Engine — Multi-Agent Orchestration

## Overview

The Quorum Engine is a distributed multi-agent LLM orchestration system that powers workflow execution in Kortecx. It coordinates parallel and sequential expert agents, manages backpressure, handles failures with recovery, and provides real-time telemetry — all over WebSocket.

---

## Architecture

```
Frontend (Next.js)                    Engine (FastAPI/Python)
┌─────────────────┐                   ┌──────────────────────────┐
│ useQuorumWS     │──WebSocket──────▸│ core/websocket.py         │
│ useWorkflowWS   │                  │   ├─ quorum.* handlers    │
│                 │                  │   └─ workflow.* handlers   │
│ Dashboard       │                  │                            │
│ Agents Page     │                  │ services/quorum/           │
│ Workflow Builder│                  │   ├─ service.py            │
│ Analytics       │                  │   ├─ scheduler.py          │
│ Monitoring      │                  │   ├─ executor.py           │
└─────────────────┘                  │   ├─ inference.py          │
        │                            │   ├─ db.py (asyncpg)      │
        ▼                            │   └─ types.py             │
   NeonDB (PostgreSQL)               │                            │
   ├─ quorum_runs                    │ services/orchestrator.py   │
   ├─ quorum_operations              │   └─ AgentOrchestrator     │
   ├─ quorum_metrics                 │                            │
   ├─ quorum_shared_memory           │ services/execution_audit   │
   ├─ step_executions                │ services/step_artifacts    │
   └─ execution_audit                └──────────────────────────┘
```

---

## Components

### Python Engine (`engine/src/engine/services/quorum/`)

| File | Purpose |
|------|---------|
| `service.py` | Main QuorumService — wires DB, inference, executor, scheduler, WS subscriptions |
| `scheduler.py` | FIFO run queue with configurable concurrency, capacity management, metrics loop |
| `executor.py` | 3+1 phase pipeline: decompose → parallel execute → recovery → synthesize |
| `inference.py` | Distributed inference client wrapping Ollama and llama.cpp |
| `db.py` | Async PostgreSQL via asyncpg — fire-and-forget operation logging, full CRUD |
| `types.py` | 14 Pydantic V2 models (RunRequest, RunResult, AgentOutput, Operation, etc.) |
| `errors.py` | Exception hierarchy (QuorumError → Inference/Scheduler/Execution/ValidationError) |

### Frontend Hooks

| Hook | Purpose |
|------|---------|
| `useQuorumWS` | WebSocket hook for quorum engine (submit, subscribe, cancel, metrics) |
| `useWorkflowWS` | WebSocket hook for workflow execution (agent status, step metrics) |
| `useLiveMetrics` | SWR hook polling engine `/api/metrics/live` every 5s |
| `useStepExecutions` | SWR hook for step execution history |
| `useExpertStats` | SWR hook for expert performance stats |

---

## Execution Pipeline

The executor runs a **3+1 phase pipeline** for each quorum run:

### Phase 1: Decompose

The lead agent analyzes the task goal and decomposes it into discrete sub-tasks, one per worker agent. The decomposition considers:

- Expert capabilities and specializations
- Task dependencies and ordering constraints
- Optimal parallelization opportunities

### Phase 2: Parallel Execute

Worker agents execute their assigned sub-tasks concurrently, subject to the configured concurrency limit. Each agent:

1. Receives its sub-task prompt plus any shared memory context
2. Calls the inference backend (Ollama or llama.cpp)
3. Streams thinking/output events over WebSocket
4. Writes its output to shared memory for downstream agents

### Phase 3: Recovery

Failed agents are retried with exponential backoff. The recovery phase:

- Identifies which agents failed and why
- Applies retry logic (up to `QUORUM_DEFAULT_RETRIES` attempts)
- Spawns recovery agents with enriched context from successful peers
- Broadcasts `quorum.agent.recovered` events on success

### Phase 4: Synthesize

A synthesis agent aggregates all agent outputs into a final coherent result. The synthesizer:

- Reads all shared memory entries
- Produces a unified output document
- Computes aggregate metrics (total tokens, duration, cost)

---

## WebSocket Protocol

All messages use the `ws.Message` envelope:

```json
{
  "event": "quorum.run.submit",
  "channel": "quorum.<run_id>",
  "data": { ... },
  "timestamp": "2026-03-21T12:00:00Z"
}
```

### Client → Server Events

| Event | Purpose |
|-------|---------|
| `quorum.run.submit` | Submit a pipeline run |
| `quorum.run.cancel` | Cancel a running pipeline |
| `quorum.run.status` | Query run status |
| `quorum.run.list` | List all runs |
| `quorum.models.list` | List inference models |
| `quorum.models.pull` | Download a model |
| `quorum.metrics.get` | Current scheduler metrics |
| `quorum.subscribe` | Subscribe to run events |

### Server → Client Events

| Event | Purpose |
|-------|---------|
| `quorum.run.started` | Run dispatched |
| `quorum.run.complete` | Run finished with results |
| `quorum.run.failed` | Run errored |
| `quorum.phase.update` | Phase started/complete with timing |
| `quorum.agent.created` | Agent spawned |
| `quorum.agent.thinking` | Agent reasoning (with live timer) |
| `quorum.agent.output` | Agent produced output |
| `quorum.agent.recovered` | Recovery agent succeeded |
| `quorum.metrics.snapshot` | Periodic system metrics (every 5s) |

### Event Payload Examples

**Run Submit:**
```json
{
  "event": "quorum.run.submit",
  "data": {
    "goal": "Research and write a technical blog post about WebAssembly",
    "experts": ["research-analyst", "content-strategist", "code-architect"],
    "mode": "parallel",
    "workers": 3,
    "retries": 3
  }
}
```

**Agent Thinking:**
```json
{
  "event": "quorum.agent.thinking",
  "channel": "quorum.run-abc123",
  "data": {
    "agentId": "agent-001",
    "expertId": "research-analyst",
    "phase": "execute",
    "elapsedMs": 2340
  }
}
```

**Run Complete:**
```json
{
  "event": "quorum.run.complete",
  "channel": "quorum.run-abc123",
  "data": {
    "runId": "run-abc123",
    "status": "completed",
    "totalTokens": 14500,
    "totalDurationMs": 8920,
    "agentCount": 3,
    "output": "..."
  }
}
```

---

## Database Schema

Five tables in `kortecx_dev` (prefixed with `quorum_`):

### `quorum_runs`

| Column | Type | Description |
|--------|------|-------------|
| id | UUID | Primary key |
| goal | TEXT | Task goal |
| status | TEXT | queued / running / completed / failed / cancelled |
| phase | TEXT | Current execution phase |
| experts | JSONB | Array of expert IDs |
| config | JSONB | Run configuration (workers, retries, mode) |
| output | TEXT | Final synthesized output |
| total_tokens | INTEGER | Aggregate token count |
| total_duration_ms | INTEGER | Wall-clock duration |
| phase_timings | JSONB | Per-phase timing breakdown |
| created_at | TIMESTAMPTZ | Run creation time |
| completed_at | TIMESTAMPTZ | Run completion time |

### `quorum_operations`

| Column | Type | Description |
|--------|------|-------------|
| id | UUID | Primary key |
| run_id | UUID | Foreign key to quorum_runs |
| agent_id | TEXT | Agent identifier |
| expert_id | TEXT | Expert used |
| operation | TEXT | prompt / response / thinking / retry / error |
| content | TEXT | Operation content |
| tokens | INTEGER | Tokens consumed |
| duration_ms | INTEGER | Operation duration |
| phase | TEXT | Pipeline phase |
| created_at | TIMESTAMPTZ | Operation time |

### `quorum_metrics`

| Column | Type | Description |
|--------|------|-------------|
| id | UUID | Primary key |
| active_runs | INTEGER | Currently running pipelines |
| queued_runs | INTEGER | Runs waiting in scheduler |
| total_tokens | INTEGER | Tokens since last reset |
| tokens_per_sec | FLOAT | Current throughput |
| cpu_percent | FLOAT | System CPU usage |
| memory_mb | FLOAT | System memory usage |
| gpu_percent | FLOAT | GPU utilization (if available) |
| created_at | TIMESTAMPTZ | Snapshot time |

### `quorum_shared_memory`

| Column | Type | Description |
|--------|------|-------------|
| id | UUID | Primary key |
| run_id | UUID | Foreign key to quorum_runs |
| phase | TEXT | Pipeline phase |
| memory | JSONB | Key-value memory store snapshot |
| created_at | TIMESTAMPTZ | Snapshot time |

### `quorum_projects`

| Column | Type | Description |
|--------|------|-------------|
| id | UUID | Primary key |
| name | TEXT | Project name |
| description | TEXT | Project description |
| config | JSONB | Default run configuration |
| created_at | TIMESTAMPTZ | Creation time |
| updated_at | TIMESTAMPTZ | Last update |

---

## Backpressure & Concurrency

The Python engine uses `asyncio.Semaphore` for backpressure:

- **Parallel mode**: `asyncio.gather()` with semaphore-based concurrency limit
- **Sequential mode**: Chain execution passing output between steps via shared memory
- **Retry**: Fallback models with exponential backoff

Agents read from shared memory at the start of execution and write their outputs back on completion, enabling inter-agent communication within a run.

---

## Configuration

Engine settings in `.env`:

| Variable | Default | Description |
|----------|---------|-------------|
| `QUORUM_MAX_CONCURRENT` | 4 | Max parallel runs in the scheduler |
| `QUORUM_METRICS_INTERVAL` | 5.0 | Metrics push interval in seconds |
| `QUORUM_DEFAULT_WORKERS` | 3 | Default worker count per run |
| `QUORUM_DEFAULT_RETRIES` | 3 | Default retry limit per agent |

---

## Error Handling

The quorum engine defines a structured exception hierarchy:

| Exception | Parent | When |
|-----------|--------|------|
| `QuorumError` | `Exception` | Base for all quorum errors |
| `InferenceError` | `QuorumError` | Model inference failed (timeout, OOM, bad response) |
| `SchedulerError` | `QuorumError` | Queue full, capacity exceeded |
| `ExecutionError` | `QuorumError` | Agent execution failed after retries |
| `ValidationError` | `QuorumError` | Invalid run request or configuration |

All errors are logged to `quorum_operations` with full context and broadcast as `quorum.run.failed` events.

---

## Related Documentation

- [Expert System](EXPERT_SYSTEM.md) — Expert definitions used by quorum agents
- [Workflow Builder](WORKFLOW_BUILDER.md) — Visual interface for composing quorum runs
- [Monitoring](MONITORING.md) — Observability and metrics from quorum execution
- [MCP Servers](MCP_SERVERS.md) — Tool integrations available to agents
