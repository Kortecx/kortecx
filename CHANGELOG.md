# Changelog

All notable changes to the Kortecx platform.

## [Unreleased] — 2026-03-21

### Added

#### Intelligence Section
- Fine-tuning page — create LoRA fine-tuning jobs with local models, dataset selection, hyperparameter config
- Inference page — cloud-only feature with CTA to Kortecx Cloud, local inference via Workflow Builder
- Models page — three tabs: Local Models (full management), Kortecx Models (cloud), Advanced Models (cloud)
- Local model management — list, search, pull with SSE progress, delete across Ollama/llama.cpp
- Kortecx Models and Advanced Models redirect to https://www.kortecx.com for cloud signup
- Static timezone list (75 zones) replacing Intl.supportedValuesOf to prevent SSR hydration mismatches

#### Settings Page Rewrite
- 8-panel left navigation: General, Inference, Agents & Runs, Tokens & Budget, Logging & Metrics, Notifications, Security & API, Feature Flags
- Ollama/llama.cpp enable/disable toggles with URL configuration
- Feature flags to enable/disable platform capabilities
- All settings persist to localStorage with deep merge on load

#### Quorum Multi-Agent Engine
- 3+1 phase pipeline executor (decompose → parallel execute → recovery → synthesize)
- FIFO run scheduler with configurable concurrency (default 4)
- Async PostgreSQL persistence via asyncpg with fire-and-forget operation logging
- WebSocket event protocol with 20+ event types for real-time telemetry
- Go client SDK with typed methods, generic callbacks, and `ParseData[T]` helper
- Client-side orchestrator with semaphore-based backpressure
- Thread-safe shared memory store (`RWMutex`) for inter-agent context
- Exponential backoff retry with configurable limits

#### Expert Marketplace
- 12 prebuilt experts: Code Architect, Research Analyst, Content Strategist, Data Engineer, Marketing Growth, CRM Specialist, Support Agent, Social Media Auto, Security Auditor, QA Reviewer, Legal Advisor, Financial Analyst
- Per-file versioning (only changed files versioned, no deep cloning)
- Expert management REST API with CRUD, version history, and restore
- Local expert creation with custom prompts and configuration

#### Workflow Builder Enhancements
- Monaco editor for system prompts and user prompts (vs-dark theme)
- Locked/editable system prompt with toggle button
- Markdown preview dialogs for prompts and task goals
- Model selector dropdown with installed model list, search, and pull-from-registry with SSE streaming progress
- MCP Servers tab in integration selector
- Editable step name and description fields
- Colored execution toggles (Sequential/Parallel, Shared/Isolated memory) with tooltips
- Workflow configuration dialog (Config, Schedule, Triggers, Notifications tabs)
- Live execution status overlay on step cards (status, timer, CPU/GPU, tokens)

#### Execution Artifacts
- Automatic artifact persistence to `steps/execution/{workflow}/{step}/`
- Code block extraction from LLM responses, saved as executable scripts
- Async script execution with timeout and result capture
- Script stdout fed back into agent memory for subsequent steps
- Structured failure logging with full context

#### Monitoring & Analytics
- Execution audit trail persisted to NeonDB (full prompts, responses, tokens, timing)
- Step execution metrics table (status, resource usage, timing breakdowns)
- Metrics auto-capture (snapshots persisted when stale >1 minute)
- Enterprise analytics dashboard (workflow performance, system health, activity feed)
- Live engine metrics on dashboard and agents pages
- Log persistence from orchestrator to frontend DB

#### Frontend Infrastructure
- `useQuorumWS` hook for quorum WebSocket communication
- `useLiveMetrics`, `useStepExecutions`, `useExpertStats`, `useMetricsHistory` SWR hooks
- `stepExecutions`, `executionAudit`, `modelComparisons` DB tables
- Model comparison re-run API (`/api/workflows/rerun`)
- Step execution history API (`/api/workflows/executions`)
- Streaming model pull endpoint (`/api/orchestrator/models/pull/stream`)

#### Go Client
- `quorum/` package with 30+ event constants and typed service methods
- Generic `Callbacks` struct with `RegisterCallbacks` for type-safe event handling
- `Orchestrator` with `ExecuteParallel`, `ExecuteSequential`, and `SharedMemoryStore`
- Expert workflow types (`ExpertRunRequest`, `ExpertStepConfig`, `LocalModelConfig`)
- Live metrics, audit operation, and model comparison types
