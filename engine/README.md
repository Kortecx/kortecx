# Kortecx Engine

Python FastAPI backend powering the Kortecx platform — agent orchestration, local and cloud inference, model training, data engineering, and MCP server management.

---

## Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| **Python** | 3.11+ | Runtime for the engine |
| **uv** | latest | Python package manager — [docs.astral.sh/uv](https://docs.astral.sh/uv/) |
| **Docker services** | Running | PostgreSQL, Qdrant must be available (via `docker compose up -d` from root) |

### Optional

| Tool | Purpose |
|------|---------|
| **Ollama** | Local LLM inference (recommended) — [ollama.com](https://ollama.com/) |
| **llama.cpp** | Alternative local inference with parallel execution |
| **CUDA/GPU** | Accelerated training and inference |

---

## Setup

```bash
# From the engine/ directory
cp .env.example .env      # Configure environment
uv sync                   # Install dependencies
```

### Start

```bash
uv run uvicorn engine.main:app --host 0.0.0.0 --port 8000 --reload
```

Or from the root: `make engine`

The API will be available at `http://localhost:8000`. Interactive docs at `http://localhost:8000/docs`.

---

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `HOST` | `0.0.0.0` | Server bind address |
| `PORT` | `8000` | Server port |
| `DEBUG` | `false` | Enable debug logging |
| `DATABASE_URL` | `postgresql://kortecx:kortecx@localhost:5433/kortecx_dev` | PostgreSQL connection |
| `DUCKDB_PATH` | `:memory:` | DuckDB path (`:memory:` for ephemeral, file path for persistent) |
| `QDRANT_URL` | `http://localhost:6333` | Qdrant vector database |
| `QDRANT_COLLECTION` | `kortecx_embeddings` | Default Qdrant collection name |
| `HF_TOKEN` | *(empty)* | HuggingFace token for gated models |
| `SPARK_MASTER` | `local[*]` | Apache Spark master URL |
| `OLLAMA_URL` | `http://localhost:11434` | Ollama inference server |
| `LLAMACPP_URL` | `http://localhost:8080` | llama.cpp server |
| `UPLOAD_DIR` | `./uploads` | File upload directory |
| `MAX_CONCURRENT_AGENTS` | `10` | Max parallel agent executions |
| `AGENT_RETRY_ENABLED` | `true` | Auto-retry failed agents with fallback model |
| `AGENT_FALLBACK_MODEL` | `llama3.2:3b` | Fallback model for retries |
| `DEFAULT_LOCAL_ENGINE` | `ollama` | Default local inference engine |
| `DEFAULT_LOCAL_MODEL` | `llama3.1:8b` | Default local model |

---

## Key Directories

```
engine/
├── src/engine/
│   ├── main.py              # FastAPI app entry point
│   ├── config.py            # Settings via pydantic-settings
│   ├── core/
│   │   └── websocket.py     # WebSocket manager for real-time events
│   ├── routers/             # API route handlers
│   │   ├── orchestrator.py  # Workflow execution endpoints
│   │   ├── experts.py       # Expert CRUD and execution
│   │   ├── training.py      # Model fine-tuning
│   │   ├── inference.py     # Model inference
│   │   ├── data.py          # DuckDB queries
│   │   ├── synthesis.py     # Data synthesis jobs
│   │   ├── mcp.py           # MCP server management
│   │   ├── quorum.py        # Multi-agent quorum engine
│   │   └── ...              # embeddings, lineage, metrics, models, providers
│   └── services/            # Business logic
│       ├── orchestrator.py  # Agent orchestration runtime
│       ├── action_runner.py # Action step execution (file generation, Docker containers)
│       ├── local_inference.py # Ollama & llama.cpp inference
│       ├── mcp.py           # MCP server discovery, caching, execution
│       ├── expert_manager.py # Expert CRUD with versioning
│       ├── step_artifacts.py # Execution artifact persistence
│       ├── synthesis.py     # Data synthesis pipeline
│       ├── execution_audit.py # Audit trail logging
│       ├── duckdb.py        # DuckDB analytical engine
│       ├── qdrant.py        # Vector search
│       ├── spark.py         # PySpark integration
│       └── ...              # hf, mlflow_tracker, system_stats
├── tests/                   # pytest test suite (205 tests)
├── mcp/                     # Prebuilt MCP server scripts
├── mcp_scripts/             # User-persisted MCP scripts
├── experts/                 # Expert artifacts
│   ├── marketplace/         # 12 prebuilt experts
│   └── local/               # User-created expert outputs
├── steps/                   # Workflow step execution artifacts
│   └── execution/           # Per-workflow/step output files
├── uploads/                 # User file uploads
└── pyproject.toml           # Dependencies and tool config
```

---

## API Endpoints

FastAPI auto-generates interactive docs at `http://localhost:8000/docs`.

| Router | Path Prefix | Purpose |
|--------|-------------|---------|
| Orchestrator | `/api/orchestrator` | Workflow execution, file uploads, run status |
| Experts | `/api/experts` | Expert CRUD, execution, marketplace |
| Training | `/api/training` | Fine-tuning jobs (SFT, DPO, RLHF, ORPO) |
| Inference | `/api/inference` | Model inference and chat |
| Data | `/api/data` | DuckDB queries and mutations |
| Synthesis | `/api/synthesis` | Data generation jobs |
| MCP | `/api/mcp` | MCP server lifecycle (discover, cache, test, persist) |
| Quorum | `/api/quorum` | Multi-agent quorum engine |
| Embeddings | `/api/embeddings` | Vector search via Qdrant |
| Models | `/api/models` | Model registry (local + cloud) |
| Providers | `/api/providers` | AI provider management |
| Metrics | `/api/metrics` | System metrics and analytics |
| Lineage | `/api/lineage` | Data lineage tracking |

---

## Services

| Service | Description |
|---------|-------------|
| **orchestrator** | Core workflow runtime — spawns agents, manages shared memory, coordinates sequential/parallel/conditional execution |
| **action_runner** | Action step execution — generates markdown/PDF files, runs MCP scripts and executables inside Docker containers |
| **local_inference** | Ollama and llama.cpp inference with model pool management and automatic fallback |
| **mcp** | MCP server discovery (prebuilt + persisted + cached), testing, and AI-assisted generation |
| **expert_manager** | Expert CRUD with per-file versioning, marketplace sync, and artifact management |
| **step_artifacts** | Persists execution outputs, scripts, and context to disk per workflow step |
| **synthesis** | Data generation pipeline (JSONL, CSV, Alpaca, ChatML, ShareGPT formats) |
| **execution_audit** | Structured audit trail for all agent spawns, completions, and failures |
| **duckdb** | Analytical SQL engine for querying datasets |
| **qdrant** | Vector similarity search for RAG and embeddings |

---

## Docker Executor Containers

The engine uses two sandboxed Docker containers for executing scripts safely:

| Container | Image | Purpose | Packages |
|-----------|-------|---------|----------|
| `kortecx_executor_python` | `python:3.11-slim` | Python script execution | numpy, pandas, requests, httpx, pydantic, duckdb |
| `kortecx_executor_ts` | `node:20-slim` | TypeScript/JS execution | tsx, esbuild, react, react-dom |

Both containers:
- Mount `./engine/steps:/steps:ro` (read-only access to step artifacts)
- Mount `./engine/steps/execution:/output` (writable, for action step file output)
- Have resource limits: 2GB RAM, 2 CPUs
- Run continuously (`tail -f /dev/null`) and execute scripts via `docker exec`

Action steps and MCP transformers route scripts to the appropriate container based on file extension (`.py` → Python, `.ts`/`.js` → TypeScript).

---

## Testing

```bash
uv run pytest                    # Run all tests (205 tests)
uv run pytest -x                 # Stop on first failure
uv run pytest --cov              # With coverage
uv run pytest tests/test_quorum.py  # Run specific test file
```

### Linting

```bash
uv run ruff check src/           # Check for issues
uv run ruff check --fix src/     # Auto-fix
```

Ruff is configured in `pyproject.toml`: Python 3.11 target, 170 char line length, isort + pyflakes + pycodestyle rules.

---

## Tech Stack

| Library | Version | Purpose |
|---------|---------|---------|
| FastAPI | 0.115+ | Async web framework |
| Uvicorn | 0.34+ | ASGI server |
| PyTorch | 2.5+ | ML framework |
| Transformers | 4.48+ | Model inference and training |
| Unsloth | 2025.3+ | LoRA training acceleration |
| TRL | 0.15+ | Training (SFT, DPO, RLHF) |
| Peft | 0.14+ | Parameter-efficient fine-tuning |
| LangChain | 0.3+ | Agent orchestration utilities |
| DuckDB | 1.2+ | Analytical SQL engine |
| PySpark | 3.5+ | Distributed data processing |
| Qdrant Client | 1.13+ | Vector database |
| MLflow | 2.18+ | Experiment tracking |
| httpx | 0.27+ | Async HTTP client |
| fpdf2 | 2.7+ | PDF generation (action steps) |
| Pydantic | 2.10+ | Data validation and settings |
