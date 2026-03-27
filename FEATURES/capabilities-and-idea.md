# Kortecx Platform — Capabilities & Idea Pipeline

## Vision

Kortecx is an agentic AI orchestration platform where **PRISM bundles** (Prompt, References, Instructions, Scripts, Models) are the atomic units of intelligence. PRISMs are connected via vector similarity in a graph structure, and execute collaboratively via **Plans** — DAG-based or stateless configurations that define multi-agent workflows with parallel execution, dependency resolution, and result aggregation.

---

## Core Architecture

### PRISM as the Atomic Agent Unit
- Each PRISM encapsulates: system prompt, model config, role, capabilities, tags, and metadata
- PRISMs are embedded as vectors in Qdrant (`kortecx_prisms` collection) using `sentence-transformers/all-MiniLM-L6-v2`
- Similarity-based connections emerge automatically; explicit connections can be added manually from list view
- The graph structure enables discovery: models find the right PRISM chain for any task
- Graph view is read-only (view, understand relationships); connection management happens in list view

### Plan as the Execution Blueprint
- A **Plan** is a DAG (Directed Acyclic Graph) or stateless config that specifies:
  - Which PRISMs participate
  - The execution order and dependencies between them
  - Which PRISMs can run in parallel (no dependencies)
  - How results flow from one PRISM to the next
  - Termination conditions and error handling
- Plans can be:
  - **User-defined**: Built visually in a Plan Builder (React Flow DAG editor)
  - **Model-generated**: A meta-PRISM analyzes the goal and generates an optimal plan via Ollama/custom model
  - **Hybrid**: User sets constraints, model fills in the execution graph
- Plans are stored in NeonDB (`plans` table) and embedded in Qdrant (`kortecx_plans` collection)

### Execution Engine
- Reads the Plan DAG
- Resolves dependencies: identifies independent PRISMs → runs them in parallel
- Passes outputs as inputs to downstream PRISMs
- Tracks execution state, tokens, latency, cost per node
- Supports: retry, fallback PRISMs, conditional branching, human-in-the-loop checkpoints
- Go quorum engine handles enterprise-scale parallel coordination, logging, context and memory sharing

---

## Implemented Features

### Phase 1: PRISM Foundation (Complete)

- [x] **Experts → PRISM rename** across entire UI (nav, pages, dialogs, deploy page)
- [x] **Bundle page** for creating PRISMs with mandatory fields (name, description, role, category, tags) and collapsible advanced metadata (complexity level, capabilities, specializations)
- [x] **DB schema**: `category` and `complexity_level` columns on experts table
- [x] **Nav icon**: PRISM section uses Boxes icon
- [x] **API updates**: POST/PATCH endpoints accept new graph metadata fields
- [x] **Engine sync**: `expert_sync.py` maps new fields between engine and PostgreSQL

### Phase 2: Graph & Embedding Foundation (Complete)

- [x] **Cytoscape.js graph visualization** — view-only with vibrant role colors, adaptive layout (spreads when few nodes, clusters when many)
- [x] **List view** with sortable columns, group-by (None/Role/Category/Status), collapsible tree sections
- [x] **Connections column** in list view — shows linked PRISM names as chips, `+` button opens name autocomplete to create edges
- [x] **Qdrant vector embedding** on PRISM create/update/delete via `embedPrism()` fire-and-forget calls
- [x] **Re-embed on edit** — only triggers when graph-relevant fields change (name, description, role, category, tags, complexityLevel)
- [x] **Similarity-based edges** via cosine distance, threshold 0.15, limit 30
- [x] **Engine graph/edges endpoint** — scrolls Qdrant, computes pairwise similarity, deduplicates edges
- [x] **Batch re-embed** — `POST /api/experts/engine/embed/all` to bootstrap or refresh all PRISM vectors
- [x] **Version-based efficient polling** — lightweight `/graph/version` endpoint polled every 10s, full edges only re-fetched when version changes
- [x] **Marketplace graph view** — graph/list toggle on Marketplace tab, edges computed client-side via Jaccard similarity on specializations + capabilities
- [x] **Explicit edge creation** — `POST /api/experts/graph` calls engine attach endpoint, re-embeds both PRISMs with affinity text
- [x] **White background + black edges** in graph, dark text labels, clean toolbar/legend styling
- [x] **connectTo flow** — Bundle page reads `?connectTo=` URL param, shows connection banner, auto-attaches after deploy

### Phase 3: Unified RUNS Page & React Flow DAGs (Complete)

- [x] **Nav restructure** — removed "PRISM & Tasks" from PRISM section, removed "Run History" from WORKFLOW section, added "Runs" as first item in MONITORING section
- [x] **Unified RUNS page** (`/monitoring/runs`) combining expert runs + workflow runs
- [x] **Metrics bar**: Running, Completed, Failed, Total Tokens — auto-refreshing
- [x] **Two tabs**: All Runs (unified sortable list with expand) and Graphs (React Flow DAG)
- [x] **Run list**: type badge (PR/WF), name, status, duration, tokens, started time, expandable details, "View Graph" button
- [x] **React Flow integration** (`@xyflow/react`) with custom `RunGraphNode` component
  - Nodes colored by status: gray (pending), blue pulse (running), green (completed), red (failed)
  - Shows role emoji, PRISM name, status badge, tokens, duration per node
  - Arrow edges with animation for active data flow
  - White background, dot grid, minimap, controls
- [x] **Edit mode**: toggle to drag nodes, add/remove connections via React Flow handles
- [x] **Save**: persists updated DAG node positions and edges
- [x] **Plans data model**: `plans` table (id, workflowId, name, dag JSONB, status, generatedBy, modelUsed)
- [x] **Plans CRUD API**: GET/POST/PATCH/DELETE at `/api/plans`
- [x] **`planId` on workflow_runs** for linking runs to their execution plan
- [x] **Types**: `Plan`, `PlanNode`, `PlanEdge`, `UnifiedRun` in types.ts
- [x] **`usePlans()` hook** for SWR data fetching

---

## Feature Pipeline

### Phase 4: Plan Generation & Execution
**Status: Next**

#### Auto-Plan Generation
- Meta-PRISM: given a goal statement, decomposes it into sub-tasks
- Maps sub-tasks to existing PRISMs (via Qdrant similarity search)
- Generates a DAG with parallelism where possible
- User reviews and approves before execution
- Endpoint: `POST /api/orchestrator/plan/generate`

#### Plan Execution Engine
- Topological sort of DAG → identify execution waves (groups of independent nodes)
- Wave 1: all root nodes (no dependencies) → run in parallel via `asyncio.gather()`
- Wave 2: nodes whose dependencies are all complete → run in parallel
- Each node execution: call the PRISM's model with system prompt + input from upstream
- Result aggregation: downstream nodes receive concatenated/structured outputs from all parents
- Go quorum engine (`go-client/quorum/orchestrator.go`) for enterprise-scale parallel execution with `ExecuteParallel`, shared memory, retry, backpressure

#### Batch Runs
- Trigger 10 runs from workflow page using same plan
- Execute all via go-client `ExecuteParallel` (MaxParallel=10)
- Each run gets its own WebSocket channel for live updates
- RUNS page shows 10 graphs in a grid, each updating independently

#### Plan Builder UI
- Visual DAG editor using React Flow
- Drag PRISMs from a sidebar onto the canvas
- Connect them with edges to define data flow
- Configure per-node overrides (temperature, max tokens, custom prompt injection)
- Validate: cycle detection, unreachable nodes, missing PRISMs

### Phase 5: Agentic Coordination
**Status: Pipeline**

- **Inter-PRISM messaging**: PRISMs can request help from other PRISMs mid-execution
- **Dynamic plan modification**: Running plan can spawn new nodes based on intermediate results
- **Quorum decisions**: Multiple PRISMs vote on a decision, majority wins
- **Escalation**: If a PRISM fails or is uncertain, escalate to a higher-capability PRISM
- **Memory sharing**: Shared context store (Qdrant + PostgreSQL) across PRISMs in a plan
- **Tool use**: PRISMs can invoke MCP servers, APIs, databases during execution

### Phase 6: Plan Templates & Scheduling
**Status: Pipeline**

- **Plan templates**: Save successful plans as reusable templates
- **Scheduled execution**: Cron-based plan triggers (daily report generation, monitoring, etc.)
- **Parameterized plans**: Plans with input variables that change per run
- **Plan versioning**: Track plan evolution, diff between versions
- **Plan marketplace**: Share and discover community plans

### Phase 7: Observability & Analytics
**Status: Pipeline**

- **Execution timeline**: Gantt chart of plan execution showing parallel/sequential phases
- **Cost attribution**: Per-PRISM and per-plan cost tracking
- **Performance optimization**: Identify bottleneck PRISMs, suggest parallelization opportunities
- **Quality scoring**: Rate plan outputs, feed back into PRISM selection
- **Drift detection**: Alert when PRISM outputs change significantly over time

### Phase 8: Advanced Graph Intelligence
**Status: Pipeline**

- **Auto-clustering**: Qdrant similarity automatically groups PRISMs into functional clusters
- **Graph-based PRISM recommendation**: "Users who used this PRISM also used..."
- **Capability gap analysis**: Identify missing PRISMs for common task patterns
- **Graph evolution tracking**: How the PRISM graph changes over time
- **Cross-plan analysis**: Which PRISMs are most reused, most effective in chains

---

## Technical Architecture

### Stack
| Layer | Technology |
|-------|-----------|
| Frontend | Next.js 16 + React 19 + TypeScript 5 |
| Graph viz (PRISM) | Cytoscape.js + cytoscape-fcose |
| Graph viz (Plans/Runs) | @xyflow/react (React Flow) |
| Animation | Framer Motion |
| Database | NeonDB (PostgreSQL 16) via Drizzle ORM |
| Vector DB | Qdrant (Docker, cosine distance) |
| Embedding | HuggingFace `sentence-transformers/all-MiniLM-L6-v2` (768-d) |
| Backend | FastAPI (Python 3.11+) |
| Orchestration | Go quorum engine (`go-client/`) |
| Real-time | WebSocket pub/sub |
| Local inference | Ollama / llama.cpp via inference router |
| Testing | Vitest (frontend), Pytest (backend) |
| Linting | ESLint + Ruff |

### Qdrant Collections
| Collection | Purpose |
|-----------|---------|
| `kortecx_embeddings` | General text embeddings |
| `kortecx_prisms` | PRISM metadata vectors for graph similarity |
| `kortecx_plans` | Plan description vectors for plan retrieval/reuse (pipeline) |

### Key Data Tables
| Table | Purpose |
|-------|---------|
| `experts` | PRISM definitions with graph metadata (category, complexityLevel) |
| `plans` | DAG execution blueprints (JSONB dag field) |
| `workflow_runs` | Workflow execution records (linked to planId) |
| `expert_runs` | Individual PRISM execution records |
| `step_executions` | Per-step telemetry (tokens, latency, CPU/GPU) |

### Embedding Pipeline
- Trigger: On PRISM create, update (graph-relevant fields), delete, attach
- Text: `{name}. {description}. Role: {role}. Category: {category}. Tags: {tags}. Capabilities: {capabilities}`
- Edge threshold: cosine > 0.15 (lowered for more visible connections)
- Efficient polling: version endpoint every 10s, full edges only on version change

### Execution Parallelism
- Python `asyncio.gather()` for parallel PRISM execution within a wave
- Go quorum engine for distributed coordination with shared memory, backpressure, retries
- WebSocket for real-time execution status updates to frontend
- React Flow nodes update live: pending → running → completed/failed

### Data Flow in Plans
- **Pass-through**: Output of PRISM A becomes input context for PRISM B
- **Aggregation**: Multiple parent outputs merged (concatenation, structured JSON, or summary)
- **Filtering**: Conditional edges — only pass output if it meets criteria
- **Branching**: Output determines which downstream path to take

---

## Immediate Next Steps

1. Plan generation endpoint — Ollama/model generates DAG from goal + available PRISMs
2. Plan execution engine — topological sort → parallel waves → result aggregation
3. Wire workflow "Run" button → plan generation → execution → live graph updates
4. Batch run support (10 parallel runs from same plan)
5. Qdrant plan embeddings for retrieval and reuse
6. Plan Builder UI — drag-and-drop DAG editor with React Flow
