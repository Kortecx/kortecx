<h1 align="center" style="border-bottom: none">
    <a href="https://kortecx.com/">
        <img alt="Kortecx logo" src="frontend/public/kortecx_readme.png" width="400" />
    </a>
</h1>
<h2 align="center" style="border-bottom: none">Executable Intelligence Platform</h2>

<p align="center">
Open-source platform for orchestrating AI agents and building agentic workflows — local or cloud.
</p>

<div align="center">

[![License](https://img.shields.io/github/license/kortecx/kortecx)](LICENSE)
<a href="https://twitter.com/intent/follow?screen_name=kortecx" target="_blank">
<img src="https://img.shields.io/twitter/follow/kortecx?logo=X&color=%20%23f5f5f5" alt="follow on X(Twitter)"></a>
<a href="https://www.linkedin.com/company/kortecx/" target="_blank">
<img src="https://custom-icon-badges.demolab.com/badge/LinkedIn-0A66C2?logo=linkedin-white&logoColor=fff" alt="follow on LinkedIn"></a>

</div>

<div align="center">
   <a href="https://kortecx.com/"><strong>Website</strong></a> ·
   <a href="https://kortecx.com/docs"><strong>Docs</strong></a> ·
   <a href="https://github.com/Kortecx/kortecx/issues/new/choose"><strong>Feature Request</strong></a> ·
   <a href="https://www.youtube.com/@executableintelligence"><strong>YouTube</strong></a>
</div>

---

## Philosophy

**Executable Intelligence** — AI agents that produce tangible outputs (reports, datasets, code, artifacts), not just chat responses. Every workflow run generates files, metrics, and lineage you can inspect, version, and reuse.

- **Local-first, cloud-optional** — Models run on your hardware by default via Ollama or llama.cpp. Cloud providers (Anthropic, OpenAI, Google, Groq, Mistral, OpenRouter) are opt-in, never required.
- **Open & self-hostable** — MIT license, no authentication wall, no vendor lock-in. Run on a laptop or deploy to your own infrastructure. PostgreSQL via Docker or Neon cloud.
- **Orchestration, not framework** — Kortecx is the control plane for chaining AI experts into workflows. It is not an SDK for building agents — it is where agents get work done.
- **Data lineage & durability** — Every execution is tracked. Artifacts are versioned. Schema migrations are managed. Backups run automatically. Nothing is lost.

---

## What It Does

- **Workflow Builder** — Chain AI agents (sequential, parallel, conditional) with shared memory, file attachments, integrations, action steps, and plugins per step
- **Dual Inference** — Run models locally (Ollama / llama.cpp) or via cloud providers (Anthropic, OpenAI, Google, Groq, Mistral, OpenRouter)
- **Expert System** — Bundle custom agents / use prebuilt marketplace experts, deployable with custom system prompts and performance tracking
- **Intelligence** — Fine-tune local models with LoRA, manage local/cloud model registries, cloud inference via [Kortecx Cloud](https://www.kortecx.com)
- **Data Engineering** — SQL analytics (DuckDB, PySpark), vector search (Qdrant), HuggingFace Hub integration
- **Connections** — Plug in external APIs, databases, tools, and marketplace plugins to any workflow step
- **Monitoring** — Real-time metrics, structured logs, alerts, cost tracking

---

## Prerequisites

### Required

| Tool | Version | Purpose | Install |
|------|---------|---------|---------|
| **Docker** + Docker Compose | 20.10+ / v2+ | Runs PostgreSQL, Qdrant, MLflow, sandboxed executor containers | [docker.com](https://docs.docker.com/get-docker/) |
| **Node.js** | 20+ | Frontend (Next.js 16, React 19) | [nodejs.org](https://nodejs.org/) or `nvm install 20` |
| **Python** | 3.11+ | Engine (FastAPI, PyTorch, Transformers) | [python.org](https://www.python.org/) or `pyenv install 3.11` |
| **uv** | latest | Fast Python package manager (replaces pip/poetry) | [docs.astral.sh/uv](https://docs.astral.sh/uv/) |
| **Git** | 2.x+ | Source control | [git-scm.com](https://git-scm.com/) |

---

## Quick Start

```bash
git clone https://github.com/Kortecx/kortecx.git
cd kortecx
cp .env.example .env
./start.sh
```

Open [http://localhost:3000](http://localhost:3000) — you should see the Kortecx dashboard.

> `start.sh` handles everything: Docker services, dependency installation, database migrations, and starting both the engine and frontend.

---

## Setup (Detailed)

### 1. Clone the repository

```bash
git clone https://github.com/Kortecx/kortecx.git
cd kortecx
```

### 2. Configure environment

```bash
cp .env.example .env
```

Edit `.env` if needed. Key variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | `postgresql://kortecx:kortecx@localhost:5433/kortecx_dev` | PostgreSQL connection string |
| `DB_MODE` | `local` | Set to empty for Neon cloud DB |
| `QDRANT_URL` | `http://localhost:6333` | Qdrant vector database |
| `NEXT_PUBLIC_ENGINE_URL` | `http://localhost:8000` | Engine URL for frontend |

### 3. Start everything

```bash
./start.sh
```

### 4. Verify

Open [http://localhost:3000](http://localhost:3000) — you should see the Kortecx dashboard.

Check Docker services are running:
```bash
docker compose ps
```

You should see: `kortecx_db`, `kortecx_qdrant`, `kortecx_mlflow`, `kortecx_executor_python`, `kortecx_executor_ts`.

### Alternative: Neon Cloud Database

For [Neon](https://neon.tech) instead of local PostgreSQL, set in `.env`:
```
DATABASE_URL=postgresql://user:pass@ep-xxx.us-east-2.aws.neon.tech/neondb?sslmode=require
DB_MODE=
```

### Alternative: Manual Start

```bash
make docker-up          # Start Docker services
make install            # Install frontend + engine dependencies
make db-push            # Apply database schema
make engine &           # Start FastAPI engine (port 8000)
make frontend           # Start Next.js frontend (port 3000)
```

---

## Usage

### Workflow Builder
Create multi-step workflows from the **Workflow** page. Add steps, assign agents, configure prompts with the Monaco editor, choose inference source (local or cloud), and execute. Each step produces versioned artifacts (responses, extracted scripts, metrics) stored with full lineage.

### Quorum Runs
Launch multi-agent orchestration from the **Agents** page. Define a goal, select worker agents, and Kortecx decomposes the task, runs agents in parallel, handles failures with retry, and synthesizes a final output. Monitor progress in real time via WebSocket telemetry.

### Agents Management
Browse the **12 prebuilt marketplace agents** (coding, research, marketing, data engineering, security, legal, finance, etc.) or create custom agents with your own system prompts, model config, and temperature settings. All agents support per-file versioning and semantic search via Qdrant.

### Model Management
Pull and manage local models from the **Intelligence** page. Switch between Ollama and llama.cpp engines. Connect cloud providers (Anthropic, OpenAI, Google, Groq, Mistral, OpenRouter) from **Settings > Inference**. Fine-tune local models with LoRA from the Intelligence tab.

### MCP Servers
Generate, edit, test, and persist Model Context Protocol scripts from the **Providers > MCP** page. Scripts are AI-generated with streaming, testable in-browser, and version-controlled on disk.

### Quick Check
Platform-aware Q&A from the dashboard. Combines your platform context (workflows, agents, runs, datasets) with Qdrant semantic search and local inference to answer questions about your Kortecx instance.

### Common Commands

| Command | Description |
|---------|-------------|
| `make start` | Full-stack bootstrap |
| `make docker-up` / `make docker-down` | Start / stop Docker services |
| `make frontend` | Start Next.js dev server (port 3000) |
| `make engine` | Start FastAPI engine (port 8000) |
| `make install` | Install all dependencies |
| `make db-push` | Apply database migrations |
| `make backup` / `make restore` | Database backup and restore |
| `make check` | Run linting + tests + build |
| `make clean-slate` | Reset user data (preserves schema + marketplace) |

---

## How it Works

```
┌─────────────┐     ┌─────────────┐     ┌──────────────────┐
│  Frontend    │────▶│   Engine    │────▶│   Inference       │
│  Next.js 16  │◀────│  FastAPI    │◀────│  Ollama / Cloud   │
│  port 3000   │ WS  │  port 8000  │     │  port 11434       │
└─────────────┘     └──────┬──────┘     └──────────────────┘
                           │
              ┌────────────┼────────────┐
              ▼            ▼            ▼
        ┌──────────┐ ┌──────────┐ ┌──────────┐
        │PostgreSQL│ │  Qdrant  │ │  MLflow  │
        │ port 5433│ │ port 6333│ │ port 5050│
        └──────────┘ └──────────┘ └──────────┘
```

**Workflow Execution**: Each step sends a prompt through the inference layer, receives a response, extracts code blocks into executable scripts, runs them in sandboxed Docker containers, and persists all artifacts (prompts, responses, scripts, results, metrics) with full lineage tracking.

**Quorum Pipeline**: The multi-agent engine follows a 3+1 phase pattern — **Decompose** (lead agent breaks task into sub-tasks) → **Parallel Execute** (workers run concurrently with shared memory) → **Recovery** (failed agents retry with enriched context) → **Synthesize** (final agent aggregates all outputs).

**Real-Time Updates**: WebSocket connections push live events for every phase — agent thinking, token streaming, step status changes, and system metrics (CPU, GPU, memory, throughput) every 5 seconds.

---

## Project Structure

```
kortecx/
├── frontend/              # Next.js 16 + React 19 dashboard
│   ├── app/               # Pages (workflow, agents, intelligence, monitoring, ...)
│   ├── lib/               # Hooks, DB schema (Drizzle ORM), utilities
│   └── drizzle/           # Frontend database migrations
├── engine/                # FastAPI Python backend
│   ├── src/engine/        # Core app (routers, services, core)
│   ├── agents/            # Marketplace + local agent definitions
│   ├── migrations/        # SQL schema migrations
│   ├── mcp_scripts/       # User-persisted MCP server scripts
│   └── tests/             # pytest test suite
├── docs/                  # Feature documentation
├── scripts/               # Utility scripts (backup, check, clean-slate)
├── shared_configs/        # Shared configuration templates
├── docker-compose.yml     # PostgreSQL, Qdrant, MLflow, executors
├── Makefile               # Dev commands
├── start.sh               # Full-stack bootstrap script
└── kortecx.config.json    # Platform metadata & feature flags
```

---

## Platform Support

| Category | Supported |
|----------|-----------|
| **Development OS** | macOS, Linux, Windows (WSL2) |
| **Deployment** | Any Docker-capable host |
| **Local Inference** | Ollama, llama.cpp |
| **Cloud Inference** | Anthropic, OpenAI, Google, Groq, Mistral, OpenRouter |
| **Databases** | PostgreSQL 16 (local Docker or Neon cloud), Qdrant (vector store) |
| **Experiment Tracking** | MLflow |
| **CI/CD** | GitHub Actions — lint (Ruff, ESLint, tsc), test (pytest, Vitest), build, integration |

---

## Features

### Quorum Multi-Agent Engine
Distributed orchestration with parallel/sequential execution, backpressure management, failure recovery, and real-time WebSocket telemetry. See [docs/QUORUM_ENGINE.md](docs/QUORUM_ENGINE.md).

### Expert Marketplace
12 prebuilt production-ready experts (coding, research, marketing, data engineering, legal, finance, etc.) with per-file versioning and custom expert creation. See [docs/EXPERT_SYSTEM.md](docs/EXPERT_SYSTEM.md).

### Enhanced Workflow Builder
Monaco-powered prompt editors, live execution status overlays, model pull with progress, MCP server integration, action steps for file generation, and workflow configuration dialogs. See [docs/WORKFLOW_BUILDER.md](docs/WORKFLOW_BUILDER.md).

### Enterprise Monitoring
Full observability with execution artifacts, script auto-execution, graceful failure handling, metrics auto-capture, and comprehensive analytics. See [docs/MONITORING.md](docs/MONITORING.md).

---

## License

[MIT](LICENSE)
