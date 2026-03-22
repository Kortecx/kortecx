# Workflow Builder — Feature Documentation

## Overview

The Workflow Builder is a visual drag-and-chain interface for composing multi-expert AI workflows. Each step is an agent backed by an [expert](EXPERT_SYSTEM.md), with configurable execution mode, prompts, model selection, integrations, and real-time execution monitoring. Workflows are executed by the [Quorum Engine](QUORUM_ENGINE.md) with full telemetry.

---

## Step Card Features

Each step in a workflow is represented as a card with the following editable and interactive elements.

### Editable Fields

| Field | Behavior |
|-------|----------|
| **Step Name** | Inline editable text input, transparent border on focus |
| **Step Description** | Free-text notes and context for the step |
| **Prompt** | Monaco editor with markdown syntax highlighting; preview button opens rendered markdown dialog |
| **System Prompt** | Locked by default (showing expert's prompt); unlock to edit via Monaco editor |

### Execution Toggles

Color-coded toggles with tooltips to configure step behavior:

| Toggle | Colors | Options | Description |
|--------|--------|---------|-------------|
| Execution Mode | Blue / Purple | Sequential / Parallel | Sequential runs steps one after another; parallel fans out |
| Memory Mode | Green / Grey | Shared / Isolated | Shared enables inter-agent context; isolated runs in sandbox |
| Inference Source | — | Local / Cloud | Badge showing the inference backend |

### Model Selector

The model selector provides full control over which model runs each step:

1. **Engine Toggle** — Switch between Ollama and llama.cpp backends
2. **Model Dropdown** — Fetches installed models from the engine, searchable with filter
3. **Pull from Registry** — Type a model name not yet installed, click Pull to download with streaming progress bar via SSE
4. **Auto-Select** — Downloaded model is automatically selected and the list refreshes

The model pull endpoint streams progress events:

```
GET /api/orchestrator/models/pull/stream?model=llama3.2:3b
```

Returns SSE events:
```
data: {"status": "pulling", "completed": 45, "total": 100, "digest": "sha256:..."}
data: {"status": "complete", "model": "llama3.2:3b"}
```

### Integrations

Each step can be configured with external integrations through a modal selector:

| Tab | Content |
|-----|---------|
| **APIs** | External API connections (REST endpoints, databases) |
| **Tools** | Built-in tools (web search, file operations, code execution) |
| **MCP Servers** | Model Context Protocol servers with status and language badges |

The dashed integration area on each step card triggers the modal directly — no redundant "+ Add" button.

### Live Execution Status

When a workflow runs, each step card shows real-time execution overlay:

| Element | Queued | Running | Completed | Failed |
|---------|--------|---------|-----------|--------|
| Status Bar | Blue | Amber | Green | Red |
| Border Glow | Blue pulse | Amber pulse | Green solid | Red solid |
| Timer | — | Live counting | Final time | Final time |
| CPU/GPU | — | Progress bars | Final values | Last values |
| Tokens | — | Counting | Final count | Partial count |

---

## Task Goal Section

The top of the workflow builder contains the task goal configuration:

- **Monaco editor** for markdown input (vs-dark theme, full syntax highlighting)
- **Preview button** opens a full-screen rendered markdown dialog
- **Suggest Chain** button auto-picks experts based on goal content analysis
- **File upload mode** supports drag-and-drop of `.md` and `.txt` files

The Suggest Chain feature analyzes the goal text and recommends a sequence of experts best suited to accomplish it, considering task decomposition and expert capabilities.

---

## Workflow Configuration Dialog

Available from the Workflows page via the gear icon on each workflow card:

### Overview Tab

| Setting | Description |
|---------|-------------|
| Name | Workflow display name |
| Description | Purpose and scope |
| Goal Statement | Default task goal for the workflow |

### Config Tab

| Setting | Default | Description |
|---------|---------|-------------|
| Failure Strategy | `stop` | `stop`, `continue`, or `retry` on step failure |
| Max Retries | 3 | Retry limit per step |
| Timeout | 300s | Maximum execution time |
| Concurrency | 4 | Max parallel steps |
| Priority | `normal` | `low`, `normal`, `high`, `critical` |

### Schedule Tab

| Setting | Description |
|---------|-------------|
| Cron Expression | Standard cron syntax (e.g., `0 9 * * MON-FRI`) |
| Schedule Type | One-time or recurring |
| Timezone | IANA timezone (e.g., `America/New_York`) |
| Next Run | Computed next execution time |

### Triggers Tab

| Setting | Description |
|---------|-------------|
| API Trigger | Auto-generated endpoint URL for programmatic execution |
| Webhook URL | Incoming webhook that triggers the workflow |
| Trigger Payload | Expected JSON payload schema |

### Notifications Tab

| Setting | Description |
|---------|-------------|
| Email on Complete | Send email when workflow finishes successfully |
| Email on Failure | Send email when workflow fails |
| Recipients | Comma-separated email addresses |
| Slack Webhook | Slack incoming webhook URL for notifications |

---

## Execution Artifacts

Every step writes artifacts to a structured directory on disk:

```
engine/steps/execution/{workflow_name}/{step_name}/
```

### Artifact Types

| File Pattern | Content |
|--------------|---------|
| `response_*.md` | Full LLM response |
| `prompt_*.md` | User prompt sent to the model |
| `system_*.md` | System prompt sent to the model |
| `context_*.json` | Run metadata (tokens, duration, model, agent) |
| `scripts/script_*.{py,sh,js}` | Extracted code blocks |
| `*_result.json` | Script execution results (stdout, stderr, exit code) |
| `failure_*.json` | Structured failure records |
| `config.json` | Step configuration snapshot |

### Script Execution

Code blocks in LLM responses are automatically extracted, saved as executable scripts, and run programmatically:

1. **Extraction** — Code fences with language tags (````python`, ````bash`, ````javascript`) are parsed from the response
2. **Saving** — Each code block is saved as `scripts/script_{index}.{ext}`
3. **Execution** — Scripts are run with a configurable timeout in an isolated subprocess
4. **Capture** — Stdout, stderr, and exit code are saved to `*_result.json`
5. **Feedback** — Script stdout is fed back into agent shared memory for subsequent steps

This enables agents to produce and validate code within a single workflow run.

### Context File Format

```json
{
  "runId": "run-abc123",
  "stepId": "step-001",
  "agentId": "agent-001",
  "expertId": "marketplace-code-architect",
  "model": "llama3.2:3b",
  "engine": "ollama",
  "tokensUsed": 2450,
  "durationMs": 3200,
  "timestamp": "2026-03-21T12:00:00Z"
}
```

### Failure Record Format

```json
{
  "runId": "run-abc123",
  "stepId": "step-001",
  "error": "InferenceError: Model timeout after 30s",
  "attempt": 2,
  "maxRetries": 3,
  "phase": "execute",
  "agentId": "agent-001",
  "timestamp": "2026-03-21T12:00:30Z"
}
```

---

## Persistence

All step execution metrics persist to NeonDB via the `step_executions` table:

| Column | Type | Description |
|--------|------|-------------|
| id | UUID | Primary key |
| run_id | TEXT | Workflow run identifier |
| step_id | TEXT | Step identifier |
| agent_id | TEXT | Agent identifier |
| expert_id | TEXT | Expert used |
| step_name | TEXT | Human-readable step name |
| status | TEXT | pending → running → thinking → completed / failed |
| tokens_used | INTEGER | Total tokens consumed |
| duration_ms | INTEGER | Step execution duration |
| cpu_percent | FLOAT | Peak CPU usage |
| gpu_percent | FLOAT | Peak GPU usage |
| memory_mb | FLOAT | Peak memory usage |
| prompt_preview | TEXT | First 500 chars of the prompt |
| response_preview | TEXT | First 500 chars of the response |
| error_message | TEXT | Error message if failed |
| created_at | TIMESTAMPTZ | Step start time |
| completed_at | TIMESTAMPTZ | Step completion time |

### API Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/api/workflows/executions` | List step executions with filtering |
| POST | `/api/workflows/executions` | Create a step execution record |
| PATCH | `/api/workflows/executions` | Update step execution status/metrics |
| POST | `/api/workflows/rerun` | Re-run a workflow with optional model comparison |

---

## Related Documentation

- [Quorum Engine](QUORUM_ENGINE.md) — Orchestration engine powering workflow execution
- [Expert System](EXPERT_SYSTEM.md) — Expert definitions backing each workflow step
- [Monitoring](MONITORING.md) — Execution metrics and analytics
- [MCP Servers](MCP_SERVERS.md) — Tool integrations available in workflow steps
