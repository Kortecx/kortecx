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
- **Expert System** — 16 agent roles + 12 prebuilt marketplace experts, deployable with custom system prompts and performance tracking
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
| **Go 1.22+** | 1.26.1 | Parallelizm | [golang](https://go.dev/doc/install) |

---

## Setup

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
