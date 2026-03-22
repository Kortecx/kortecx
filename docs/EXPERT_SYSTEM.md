# Expert System — Marketplace & Local Experts

## Overview

Kortecx ships with 12 production-ready prebuilt experts and supports creating custom local experts. Each expert is a self-contained directory with definition, system prompt, user prompt template, and versioned artifacts. Experts serve as the agent identities that power the [Quorum Engine](QUORUM_ENGINE.md) and [Workflow Builder](WORKFLOW_BUILDER.md).

---

## Directory Structure

```
engine/experts/
├── marketplace/              # Shipped with platform (read-only)
│   ├── _registry.json        # Index of all marketplace experts
│   ├── code-architect/
│   │   ├── expert.json       # Expert definition
│   │   ├── system.md         # System prompt (15+ lines)
│   │   ├── user.md           # Default user prompt template
│   │   └── README.md         # Description
│   ├── research-analyst/
│   ├── content-strategist/
│   └── ... (12 total)
└── local/                    # User-created experts
    ├── _registry.json
    └── {expert-slug}/
        ├── expert.json
        ├── system.md
        ├── user.md
        ├── README.md
        └── .versions/        # Per-file version history
```

---

## Prebuilt Experts

| Expert | Role | Category | Temp | Tokens | Focus |
|--------|------|----------|------|--------|-------|
| Code Architect | coder | engineering | 0.3 | 8192 | Code generation, architecture, review |
| Research Analyst | researcher | research | 0.5 | 6144 | Deep research, synthesis, insights |
| Content Strategist | writer | content | 0.7 | 4096 | Blog posts, docs, SEO, marketing copy |
| Data Engineer | data-engineer | data | 0.2 | 6144 | SQL, ETL, pipelines, schema design |
| Marketing Growth | analyst | marketing | 0.6 | 4096 | Growth strategy, campaigns, funnels |
| CRM Specialist | coordinator | sales | 0.5 | 4096 | Customer journeys, lead scoring |
| Support Agent | custom | support | 0.4 | 2048 | Ticket triage, helpdesk, SLAs |
| Social Media Auto | creative | marketing | 0.8 | 2048 | Platform-specific content, scheduling |
| Security Auditor | reviewer | security | 0.2 | 6144 | Vulnerability assessment, OWASP |
| QA Reviewer | reviewer | engineering | 0.3 | 4096 | Code review, test strategy, CI/CD |
| Legal Advisor | legal | legal | 0.3 | 6144 | Contracts, compliance, GDPR/CCPA |
| Financial Analyst | financial | finance | 0.3 | 4096 | Financial modeling, forecasting, P&L |

---

## Expert Definition Format

Each expert is defined by an `expert.json` file:

```json
{
  "id": "marketplace-code-architect",
  "name": "Code Architect",
  "role": "coder",
  "version": "1.0.0",
  "modelSource": "local",
  "localModelConfig": {
    "engine": "ollama",
    "modelName": "llama3.2:3b"
  },
  "temperature": 0.3,
  "maxTokens": 8192,
  "tags": ["coding", "architecture"],
  "capabilities": ["coding", "reasoning"],
  "category": "engineering"
}
```

### Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | yes | Unique identifier (prefix: `marketplace-` or `local-`) |
| `name` | string | yes | Display name |
| `role` | string | yes | Agent role (coder, researcher, writer, reviewer, etc.) |
| `version` | string | yes | Semantic version |
| `modelSource` | string | yes | `local` or `cloud` |
| `localModelConfig` | object | if local | Engine and model name for local inference |
| `cloudModelConfig` | object | if cloud | Provider, model, and API key reference |
| `temperature` | float | yes | Sampling temperature (0.0 - 1.0) |
| `maxTokens` | integer | yes | Maximum response tokens |
| `tags` | string[] | no | Searchable tags |
| `capabilities` | string[] | no | Agent capabilities for matching |
| `category` | string | yes | Grouping category |

---

## Prompt Templates

### System Prompt (`system.md`)

The system prompt defines the expert's persona, expertise, and behavioral guidelines. Each marketplace expert ships with a detailed system prompt (15+ lines) tailored to its domain.

### User Prompt Template (`user.md`)

The user prompt template supports placeholder variables that are filled at runtime:

| Placeholder | Description |
|-------------|-------------|
| `{{task}}` | The specific task or question |
| `{{context}}` | Additional context from shared memory or previous steps |
| `{{constraints}}` | Output format constraints, length limits, etc. |

Example template:

```markdown
## Task
{{task}}

## Context
{{context}}

## Constraints
{{constraints}}

Provide a detailed, actionable response.
```

---

## Per-File Versioning

When any file in an expert is updated, ONLY that file is versioned — no deep cloning of the entire expert directory. This keeps storage efficient and makes history granular.

```
.versions/
├── system.md.v1711036800000    # Old system prompt
├── expert.json.v1711036900000  # Old config
└── user.md.v1711037000000      # Old user template
```

Version files are named `{filename}.v{unix_timestamp_ms}`. This allows:

- Tracking exactly which file changed and when
- Restoring any individual file without affecting others
- Lightweight storage (only changed files are copied)

---

## API Endpoints

All endpoints are served by the Python engine under the `/api/experts/engine/` prefix.

### List All Experts

```
GET /api/experts/engine/list
```

Returns all experts from both marketplace and local registries, merged into a single list.

**Response:**
```json
{
  "experts": [
    {
      "id": "marketplace-code-architect",
      "name": "Code Architect",
      "role": "coder",
      "source": "marketplace",
      "category": "engineering",
      ...
    }
  ]
}
```

### Get Expert Details

```
GET /api/experts/engine/{id}
```

Returns the full expert definition including prompt contents and file listings.

**Response:**
```json
{
  "expert": { ... },
  "systemPrompt": "You are a senior software architect...",
  "userPrompt": "## Task\n{{task}}\n...",
  "files": ["expert.json", "system.md", "user.md", "README.md"]
}
```

### Create Local Expert

```
POST /api/experts/engine/create
```

Creates a new expert in the `local/` directory with all required files.

**Request Body:**
```json
{
  "name": "My Custom Expert",
  "role": "analyst",
  "category": "custom",
  "temperature": 0.5,
  "maxTokens": 4096,
  "systemPrompt": "You are a specialized analyst...",
  "userPrompt": "## Task\n{{task}}"
}
```

### Update Expert File

```
POST /api/experts/engine/{id}/update
```

Updates a single file within the expert directory. The previous version is automatically saved to `.versions/`.

**Request Body:**
```json
{
  "file": "system.md",
  "content": "Updated system prompt content..."
}
```

### List File Versions

```
GET /api/experts/engine/{id}/versions/{file}
```

Returns all historical versions of a specific file.

**Response:**
```json
{
  "versions": [
    { "version": "v1711036800000", "timestamp": "2026-03-21T12:00:00Z", "sizeBytes": 1240 },
    { "version": "v1711036900000", "timestamp": "2026-03-21T12:01:40Z", "sizeBytes": 1380 }
  ]
}
```

### Restore File Version

```
POST /api/experts/engine/{id}/restore
```

Restores a file to a previous version. The current version is saved to `.versions/` before restoring.

**Request Body:**
```json
{
  "file": "system.md",
  "version": "v1711036800000"
}
```

### Delete Expert

```
DELETE /api/experts/engine/{id}
```

Deletes a local expert and all its files. Marketplace experts cannot be deleted.

---

## Frontend Integration

The frontend integrates experts through several paths:

### Workflow Builder
When adding a step to a workflow, the user selects an expert from a dropdown that lists all available experts (marketplace + local). The expert's system prompt is loaded and locked by default but can be unlocked for editing.

### Agents Page
Displays all experts with their stats: total runs, success rate, average latency, and tokens consumed. Marketplace experts show a badge indicating they are prebuilt.

### Expert Stats Hook
```typescript
const { data, isLoading } = useExpertStats();
// Returns: { expertId, runs, successRate, avgLatency, totalTokens }[]
```

---

## Creating a Custom Expert

1. Navigate to the Experts page in the frontend
2. Click "Create Expert"
3. Fill in the expert definition (name, role, category, model config)
4. Write the system prompt — this defines the agent's persona
5. Write the user prompt template with `{{task}}`, `{{context}}`, `{{constraints}}` placeholders
6. Save — the expert appears in all dropdowns and can be used in workflows

Alternatively, create experts via the API or by directly adding a directory under `engine/experts/local/` with the required files and updating `_registry.json`.

---

## Related Documentation

- [Quorum Engine](QUORUM_ENGINE.md) — How experts are orchestrated as agents
- [Workflow Builder](WORKFLOW_BUILDER.md) — Visual interface for using experts in workflows
- [Monitoring](MONITORING.md) — Expert performance tracking and analytics
