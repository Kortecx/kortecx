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

## What It Does

- **Workflow Builder** — Chain AI agents (sequential or parallel) with shared memory, file attachments, integrations, and plugins per step
- **Dual Inference** — Run models locally (Ollama / llama.cpp) or via cloud providers (Anthropic, OpenAI, Google, Groq, Mistral, OpenRouter)
- **Expert System** — 16 agent roles + 12 prebuilt marketplace experts, deployable with custom system prompts and performance tracking
- **Intelligence** — Fine-tune local models with LoRA, manage local/cloud model registries, cloud inference via [Kortecx Cloud](https://www.kortecx.com)
- **Data Engineering** — SQL analytics (DuckDB, PySpark), vector search (Qdrant), HuggingFace Hub integration
- **Connections** — Plug in external APIs, databases, tools, and marketplace plugins to any workflow step
- **Monitoring** — Real-time metrics, structured logs, alerts, cost tracking

```
┌──────────┐     ┌──────────┐     ┌──────────┐     ┌──────────┐
│ Researcher│────▶│ Analyst  │────▶│  Writer  │────▶│ Reviewer │
│ + GitHub │     │ + SQL DB │     │ + Email  │     │          │
│ + Scraper│     │ + Charts │     │  Plugin  │     │          │
└──────────┘     └──────────┘     └──────────┘     └──────────┘
                    Shared Memory (per-run KV store)
```

---

## New Features

### Quorum Multi-Agent Engine
Distributed orchestration with parallel/sequential execution, backpressure management, failure recovery, and real-time WebSocket telemetry. See [docs/QUORUM_ENGINE.md](docs/QUORUM_ENGINE.md).

### Expert Marketplace
12 prebuilt production-ready experts (coding, research, marketing, data engineering, legal, finance, etc.) with per-file versioning and custom expert creation. See [docs/EXPERT_SYSTEM.md](docs/EXPERT_SYSTEM.md).

### Enhanced Workflow Builder
Monaco-powered prompt editors, live execution status overlays, model pull with progress, MCP server integration, colored execution toggles, and workflow configuration dialogs. See [docs/WORKFLOW_BUILDER.md](docs/WORKFLOW_BUILDER.md).

### Enterprise Monitoring
Full observability with execution artifacts, script auto-execution, graceful failure handling, metrics auto-capture, and comprehensive analytics. See [docs/MONITORING.md](docs/MONITORING.md).

---

## Quick Start

**Prerequisites:** Docker, Node.js 20+, Python 3.11+, [uv](https://docs.astral.sh/uv/)

```bash
git clone https://github.com/Kortecx/kortecx.git
cd kortecx
cp .env.example .env
./start.sh
```

Open [http://localhost:3000](http://localhost:3000).

`start.sh` handles everything: Docker services (PostgreSQL + Qdrant), schema sync, engine startup, and frontend dev server.

For [Neon](https://neon.tech) cloud DB instead of local, set in `.env`:
```
DATABASE_URL=postgresql://user:pass@ep-xxx.us-east-2.aws.neon.tech/neondb?sslmode=require
DB_MODE=
```

---

## License

[MIT](LICENSE)
