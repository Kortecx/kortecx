# MCP Servers — Feature Documentation

## Overview

The MCP (Model Context Protocol) Servers feature provides a full lifecycle for creating, testing, and deploying MCP server scripts within the Kortecx platform. Users can generate scripts via AI, edit them in a VS Code-themed Monaco editor, test execution, and persist finalized scripts to disk.

---

## Architecture

```
Frontend (Next.js)                Engine (FastAPI/Python)
┌──────────────────┐              ┌──────────────────────┐
│ Connections Page  │──REST/SSE──▸│ /api/mcp/*           │
│ MCP Servers Tab   │             │                      │
│                   │             │ routers/mcp.py       │
│ Monaco Editor     │             │ services/mcp.py      │
│ Generate Dialog   │             │                      │
│ Viewer Dialog     │             │ Ollama / LlamaCpp    │
└──────────────────┘              │ Anthropic / OpenAI   │
        │                         │ Google Gemini        │
        ▼                         └──────────────────────┘
                                   engine/mcp/prompts/ (generation prompts)
                                   engine/mcp_scripts/ (persisted user scripts)
                                   engine/cache/prompts/{type}/ (cached prompts by type)
```

---

## Features

### 1. MCP Server Discovery
- **Prebuilt servers** in `engine/mcp/` — read-only, auto-discovered by file extension (.py, .ts, .js)
- **Persisted servers** in `engine/mcp_scripts/` — user-saved scripts with `.meta.json` sidecars
- **Session cache** — temporary in-memory scripts, lost on engine restart unless persisted

### 2. AI-Powered Script Generation
- **Streaming generation** via SSE — code appears token-by-token in Monaco editor
- **Multiple inference sources:**
  - Local: Ollama, LlamaCpp
  - Cloud providers: Anthropic (Claude), OpenAI (GPT), Google (Gemini), and any provider with an active API key
- **Prompt types:** MCP Server, Data Synthesis, General
- **Reactive system prompt:** Auto-configured based on selected type + language, fully editable by user
- **Generation stats:** CPU usage and wall-clock time tracked, displayed as color-coded badges:
  - Green: fast / low CPU
  - Amber: moderate
  - Red: slow / high CPU
- **Attachments:** Optional file uploads for context

### 3. Script Editing (Monaco Editor)
- **VS Code-themed** dark editor (`vs-dark` theme)
- **Multi-language support:** Python, TypeScript, JavaScript with full syntax highlighting
- **Features:** Line numbers, minimap, bracket colorization, word wrap, format-on-paste, format-on-type
- **Read-only** for prebuilt scripts, editable for cached/persisted

### 4. In-Dialog Recreate
- **Edit the generation prompt** directly in the viewer dialog (click the pencil icon)
- **Recreate** button regenerates the code in-place using the edited prompt
- Code updates reactively in the Monaco editor via streaming — no dialog close/reopen
- Uses the currently selected model, source, and system prompt from the viewer controls

### 5. Testing
- **Test button** executes the script as a subprocess (Python/Node.js/tsx)
- 30-second timeout with process kill
- Output displayed in a colored panel (green for success, red for errors)

### 6. Persistence
- **Persist button** saves a cached script to `engine/mcp_scripts/`
- Creates a `.meta.json` sidecar with: name, description, language, prompt, visibility, generation stats
- Overwrites on re-persist (no version accumulation)

### 7. Visibility & Sharing
- **Public/Private toggle** — clickable badge in viewer header
- **Share button** — copies server name, description, prompt, and full code to clipboard

### 8. Description Management
- **Editable** for cached/generated scripts (with Save button)
- **Grayed out (read-only)** for persisted scripts in the viewer dialog
- Description field available in both the create dialog and the viewer dialog

### 9. System Prompt & Model Selection in Viewer
- **System prompt** is collapsible and editable in the viewer dialog metadata bar (for both cached and persisted servers)
- **Model/Source selector** is available inline in the viewer — switch between Ollama, LlamaCpp, and connected AI providers (Anthropic, OpenAI, Google, etc.)
- When no providers are connected, a hint message directs users to the Providers tab
- Provider buttons show a colored dot + name, with visual divider separating local engines from cloud providers
- Both controls are used by the Recreate flow

### 10. Version History
- **Default 3 versions** stored per persisted script before overwriting
- Versions saved to `engine/mcp_scripts/.versions/{stem}/` with timestamped filenames
- Metadata sidecars are versioned alongside script files
- **Configurable max versions** — editable in the UI (grayed-out input, click pencil to edit)
- Old versions auto-pruned beyond the configured limit
- API: `PUT /api/mcp/config/max-versions`, `GET /api/mcp/versions/{id}`, `GET /api/mcp/config`

### 11. Cache Behavior
- **Cached files are never auto-deleted** — persisting a script keeps the cached copy
- Users must explicitly click the **Delete** button to remove a cached script
- **Copy button** on cached server cards copies the script to clipboard (name, description, prompt, code)

### 12. Prompt Caching
- All generation prompts saved to `engine/cache/prompts/{type}/` where type is:
  - `mcp` — MCP server scripts
  - `data_synthesis` — data generation scripts
  - `general` — general-purpose scripts
- MCP-type prompts also saved to `engine/mcp/prompts/` for backward compatibility
- Prompt files are Markdown with metadata headers (script name, ID, type, timestamp)

### 11. Navigation
- **MCP Servers** tab is the first tab in the Connections page (default active)
- **Left sidebar** has a dedicated MCP Servers nav item under PROVIDERS section
- Direct URL: `/providers/connections?tab=mcp`

---

## API Endpoints

### Engine (FastAPI) — prefix `/api/mcp`

| Method | Path | Description |
|--------|------|-------------|
| GET | `/servers` | List all servers (prebuilt + persisted + cached) |
| GET | `/servers/{id}` | Get single server details |
| POST | `/generate` | Generate script (non-streaming, full response) |
| POST | `/generate/stream` | Generate script (SSE streaming, token-by-token) |
| POST | `/cache` | Cache a script in session memory |
| PUT | `/cache/{id}` | Update cached script (code, description, visibility) |
| DELETE | `/cache/{id}` | Delete a cached script |
| POST | `/test/{id}` | Execute a cached script to test it |
| POST | `/persist/{id}` | Persist a cached script to disk |
| DELETE | `/persisted/{id}` | Delete a persisted script file |
| GET | `/versions/{id}` | List version history for a persisted script |
| PUT | `/config/max-versions` | Set max versions to keep (default 3) |
| GET | `/config` | Get MCP service configuration |

### Frontend (Next.js) — prefix `/api/mcp`

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/mcp` | Proxy → engine `/api/mcp/servers` |
| POST | `/api/mcp` | Action router (generate, cache, update, test, persist, delete_cached, delete_persisted) |
| POST | `/api/mcp/stream` | SSE proxy → engine `/api/mcp/generate/stream` |

---

## Data Model

### McpServer

| Field | Type | Description |
|-------|------|-------------|
| id | string | Unique ID (`prebuilt-{stem}`, `persisted-{stem}`, `cached-{uuid}`) |
| name | string | Display name |
| description | string | User-provided or prompt-derived description |
| language | string | `python` \| `typescript` \| `javascript` |
| filename | string | Script filename |
| source | string | `prebuilt` \| `generated` \| `persisted` |
| code | string | Full script source code |
| status | string | `idle` \| `running` \| `tested` \| `error` |
| test_output | string | Last test execution output |
| created_at | string | ISO timestamp |
| prompt | string | Original generation prompt |
| is_public | boolean | Visibility flag |
| generation_time_ms | number | Generation wall-clock time in ms |
| cpu_percent | number | Average CPU usage during generation |

### GenerateMcpRequest

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| prompt | string | required | User's generation prompt |
| description | string | `""` | Optional description (defaults to prompt) |
| language | string | `python` | Target language |
| model | string | `llama3.1:8b` | Model ID |
| source | string | `ollama` | `ollama` \| `llamacpp` \| `provider` |
| provider_id | string | `""` | Cloud provider ID (when source=provider) |
| system_prompt | string | `""` | Custom system prompt (falls back to default) |
| prompt_type | string | `mcp` | Prompt category for caching |

---

## File Structure

```
engine/
├── mcp/                          # Prebuilt MCP scripts (read-only)
│   ├── hello_world.py
│   ├── file_reader.py
│   └── prompts/                  # Legacy prompt storage for MCP type
├── mcp_scripts/                  # Persisted user scripts
│   ├── *.py / *.ts / *.js        # Script files
│   ├── *.meta.json               # Metadata sidecars
│   └── .versions/                # Version history
│       └── {stem}/               # Per-script version directory
│           ├── {stem}_{ts}.py    # Timestamped version snapshots
│           └── {stem}_{ts}.py.meta.json
├── cache/
│   └── prompts/
│       ├── mcp/                  # MCP generation prompts
│       ├── data_synthesis/       # Data synthesis prompts
│       └── general/              # General prompts
└── src/engine/
    ├── routers/mcp.py            # FastAPI endpoints
    └── services/mcp.py           # Business logic

frontend/
├── app/
│   ├── api/mcp/
│   │   ├── route.ts              # REST proxy
│   │   └── stream/route.ts       # SSE streaming proxy
│   └── providers/connections/
│       └── page.tsx              # MCP tab UI
├── lib/
│   ├── types.ts                  # McpServer interface
│   ├── constants.ts              # Nav items
│   └── hooks/useApi.ts           # useProviders hook
└── components/layout/
    └── LeftNavbar.tsx             # Server icon in nav

```

---

## Environment Variables (for cloud provider generation)

| Variable | Provider |
|----------|----------|
| `ANTHROPIC_API_KEY` | Anthropic (Claude) |
| `OPENAI_API_KEY` | OpenAI (GPT) |
| `GOOGLE_API_KEY` | Google (Gemini) |
| `GROQ_API_KEY` | Groq |
| `MISTRAL_API_KEY` | Mistral AI |
| `DEEPSEEK_API_KEY` | DeepSeek |
| `XAI_API_KEY` | xAI (Grok) |

These are used by the engine when `source=provider` for streaming generation. The frontend stores encrypted API keys in the database, but the engine reads from environment variables for direct provider API calls.

---

## Prebuilt MCP Scripts

### hello_world.py
Minimal MCP server with a `greet` tool. Includes self-test assertions.

### file_reader.py
File reading and listing MCP server with path traversal protection. Tools: `read_file`, `list_files`.

---

## UI Components

### Connections Page — MCP Servers Tab
- **Prebuilt servers grid** — read-only cards with purple icons
- **My MCP Servers grid** — persisted scripts with green icons, delete buttons
- **Session Cache grid** — temporary scripts with orange icons, dashed borders, status indicators
- **Generate MCP Server** button opens the create dialog

### Generate MCP Server Dialog
- Prompt type selector (MCP, Data Synthesis, General)
- Prompt textarea
- Description input
- Collapsible system prompt (auto-configured, editable)
- File attachments (optional)
- Language selector (Python, TS, JS)
- Inference source (Ollama, LlamaCpp, connected cloud providers with brand colors)
- Model dropdown (switches per source)
- Generation stats in footer (time + CPU, color-coded)

### MCP Server Viewer/Editor Dialog
- **Header:** Server name, source badge, language badge, status badges, public/private toggle, share + edit buttons, color-coded generation stats
- **Metadata bar:** Editable generation prompt (with edit toggle), description field (grayed for persisted), collapsible system prompt, inline model/source selector
- **Monaco Editor:** VS Code dark theme, language-aware, streaming code updates
- **Test output panel:** Colored success/error display
- **Footer:** Recreate (enabled after prompt edit), Share, Test, Persist, Close buttons
