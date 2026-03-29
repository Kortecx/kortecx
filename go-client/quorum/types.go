// Package quorum provides client-side types and services for the Quorum
// parallel-agent execution engine. All types mirror the server-side Pydantic
// models and are serialised over the WebSocket transport defined in ws.
package quorum

// ── Run ─────────────────────────────────────────────

// SubmitRequest describes a new quorum run to be executed.
type SubmitRequest struct {
	Project     string   `json:"project"`
	Task        string   `json:"task"`
	Model       string   `json:"model"`
	Backend     string   `json:"backend"`
	Workers     int      `json:"workers"`
	Prompt      string   `json:"prompt,omitempty"`
	Temperature float64  `json:"temperature,omitempty"`
	MaxTokens   int      `json:"max_tokens,omitempty"`
	Retries     int      `json:"retries,omitempty"`
	Plan        *PlanDAG `json:"plan,omitempty"`
}

// PlanDAG represents an execution plan DAG with nodes and edges.
type PlanDAG struct {
	Nodes []PlanNode `json:"nodes"`
	Edges []PlanEdge `json:"edges"`
}

// PlanNode is a single node in the execution plan DAG.
type PlanNode struct {
	ID             string   `json:"id"`
	AgentID        string   `json:"agentId,omitempty"`
	Label          string   `json:"label"`
	Description    string   `json:"description,omitempty"`
	ConnectionType string   `json:"connectionType,omitempty"` // "sequential" | "parallel"
	Position       Position `json:"position,omitempty"`
}

// PlanEdge represents a dependency edge between plan nodes.
type PlanEdge struct {
	ID       string `json:"id"`
	Source   string `json:"source"`
	Target   string `json:"target"`
	Animated bool   `json:"animated,omitempty"`
}

// Position represents x/y coordinates for graph layout.
type Position struct {
	X float64 `json:"x"`
	Y float64 `json:"y"`
}

// RunID is a thin wrapper used to reference a single run by its identifier.
type RunID struct {
	RunID string `json:"run_id"`
}

// RunStatus is the current state of a quorum run.
type RunStatus struct {
	ID         string `json:"id"`
	Project    string `json:"project"`
	Task       string `json:"task"`
	Status     string `json:"status"`
	Workers    int    `json:"workers"`
	Phase      string `json:"phase,omitempty"`
	CreatedAt  string `json:"created_at"`
	StartedAt  string `json:"started_at,omitempty"`
	FinishedAt string `json:"finished_at,omitempty"`
}

// RunResult contains the final output and aggregate metrics for a completed run.
type RunResult struct {
	RunID            string `json:"run_id"`
	TotalTokens      int64  `json:"total_tokens"`
	TotalDurationMs  int64  `json:"total_duration_ms"`
	DecomposeMs      int64  `json:"decompose_ms"`
	ExecuteMs        int64  `json:"execute_ms"`
	SynthesizeMs     int64  `json:"synthesize_ms"`
	FinalOutput      string `json:"final_output"`
	WorkersSucceeded int    `json:"workers_succeeded"`
	WorkersFailed    int    `json:"workers_failed"`
	WorkersRecovered int    `json:"workers_recovered"`
}

// RunListFilter constrains which runs are returned by a list request.
type RunListFilter struct {
	Project string `json:"project,omitempty"`
	Status  string `json:"status,omitempty"`
	Limit   int    `json:"limit,omitempty"`
	Offset  int    `json:"offset,omitempty"`
}

// ── Phase ───────────────────────────────────────────

// PhaseUpdate reports progress within a single run phase (decompose, execute,
// synthesize). When Parallel is true, Speedup indicates the wall-clock gain.
type PhaseUpdate struct {
	RunID           string  `json:"run_id"`
	Phase           string  `json:"phase"`
	Status          string  `json:"status"`
	Detail          string  `json:"detail,omitempty"`
	WallClockMs     int64   `json:"wall_clock_ms,omitempty"`
	SumIndividualMs int64   `json:"sum_individual_ms,omitempty"`
	Speedup         float64 `json:"speedup,omitempty"`
	Parallel        bool    `json:"parallel,omitempty"`
}

// ── Agent ───────────────────────────────────────────

// AgentCreated is emitted when a new worker agent is spawned for a run.
type AgentCreated struct {
	RunID   string `json:"run_id"`
	AgentID string `json:"agent_id"`
	Role    string `json:"role"`
	Subtask string `json:"subtask"`
	Model   string `json:"model"`
}

// AgentThinking is emitted while an agent is reasoning about its subtask.
type AgentThinking struct {
	RunID     string `json:"run_id"`
	AgentID   string `json:"agent_id"`
	Phase     string `json:"phase"`
	Reasoning string `json:"reasoning"`
}

// AgentOutput is emitted when an agent produces output for a subtask attempt.
type AgentOutput struct {
	RunID          string `json:"run_id"`
	AgentID        string `json:"agent_id"`
	Phase          string `json:"phase"`
	TokensUsed     int64  `json:"tokens_used"`
	DurationMs     int64  `json:"duration_ms"`
	ContentPreview string `json:"content_preview"`
	Attempt        int    `json:"attempt"`
	Status         string `json:"status"`
}

// AgentFailed is emitted when an agent exhausts all retry attempts.
type AgentFailed struct {
	RunID    string `json:"run_id"`
	AgentID  string `json:"agent_id"`
	Phase    string `json:"phase"`
	Error    string `json:"error"`
	Attempts int    `json:"attempts"`
	Subtask  string `json:"subtask"`
}

// AgentRecovered is emitted when a recovery agent succeeds after the
// original agent failed.
type AgentRecovered struct {
	RunID          string `json:"run_id"`
	AgentID        string `json:"agent_id"`
	OriginalAgent  string `json:"original_agent"`
	Phase          string `json:"phase"`
	TokensUsed     int64  `json:"tokens_used"`
	DurationMs     int64  `json:"duration_ms"`
	ContentPreview string `json:"content_preview"`
}

// ── Config ──────────────────────────────────────────

// ProjectConfig carries per-project quorum configuration.
type ProjectConfig struct {
	Project string         `json:"project"`
	Config  map[string]any `json:"config,omitempty"`
}

// ── Models ──────────────────────────────────────────

// ModelsRequest filters the model list by inference backend.
type ModelsRequest struct {
	Backend string `json:"backend"`
}

// PullModelRequest asks the server to download a model for the given backend.
type PullModelRequest struct {
	Model   string `json:"model"`
	Backend string `json:"backend"`
}

// ── Metrics ─────────────────────────────────────────

// MetricsSnapshot is a point-in-time view of engine resource utilisation.
type MetricsSnapshot struct {
	ActiveRuns         int     `json:"active_runs"`
	QueuedRuns         int     `json:"queued_runs"`
	MaxConcurrent      int     `json:"max_concurrent"`
	CPUUsage           float64 `json:"cpu_usage"`
	MemoryUsageMB      float64 `json:"memory_usage_mb"`
	TokensPerSec       float64 `json:"tokens_per_sec"`
	TotalRunsCompleted int     `json:"total_runs_completed"`
	TotalTokensUsed    int64   `json:"total_tokens_used"`
	AvgRunDurationMs   int64   `json:"avg_run_duration_ms"`
}

// MetricsHistoryRequest controls how many historical snapshots to return.
type MetricsHistoryRequest struct {
	Limit int `json:"limit,omitempty"`
}

// ── Operations ──────────────────────────────────────

// OperationsFilter constrains which low-level operations are returned.
type OperationsFilter struct {
	RunID     string `json:"run_id"`
	AgentID   string `json:"agent_id,omitempty"`
	Phase     string `json:"phase,omitempty"`
	Operation string `json:"operation,omitempty"`
}

// ── Errors ──────────────────────────────────────────

// ErrorResponse is the standard error envelope returned by the quorum engine.
type ErrorResponse struct {
	Event string `json:"event"`
	Error string `json:"error"`
}

// ── Expert Workflow Execution ───────────────────────

// ExpertRunRequest submits a workflow for expert-based execution.
type ExpertRunRequest struct {
	Name          string             `json:"name"`
	WorkflowID    string             `json:"workflowId,omitempty"`
	GoalFileURL   string             `json:"goalFileUrl"`
	InputFileURLs []string           `json:"inputFileUrls,omitempty"`
	Steps         []ExpertStepConfig `json:"steps"`
	Plan          *PlanDAG           `json:"plan,omitempty"`
}

// ExpertStepConfig configures a single expert step in a workflow.
type ExpertStepConfig struct {
	StepID             string            `json:"stepId"`
	ExpertID           string            `json:"expertId,omitempty"`
	TaskDescription    string            `json:"taskDescription"`
	SystemInstructions string            `json:"systemInstructions,omitempty"`
	ModelSource        string            `json:"modelSource"` // "local" | "provider"
	LocalModel         *LocalModelConfig `json:"localModel,omitempty"`
	Temperature        float64           `json:"temperature,omitempty"`
	MaxTokens          int               `json:"maxTokens,omitempty"`
	ConnectionType     string            `json:"connectionType"` // "sequential" | "parallel" | "conditional"
	Integrations       []StepIntegration `json:"integrations,omitempty"`
}

// LocalModelConfig specifies local inference backend configuration.
type LocalModelConfig struct {
	Engine  string `json:"engine"` // "ollama" | "llamacpp"
	Model   string `json:"model"`
	BaseURL string `json:"baseUrl,omitempty"`
}

// StepIntegration attaches an integration or plugin to a workflow step.
type StepIntegration struct {
	ID          string            `json:"id"`
	Type        string            `json:"type"` // "integration" | "plugin"
	ReferenceID string            `json:"referenceId"`
	Name        string            `json:"name"`
	Icon        string            `json:"icon,omitempty"`
	Color       string            `json:"color,omitempty"`
	Config      map[string]string `json:"config,omitempty"`
}

// ExpertStatsUpdate is pushed when expert performance stats are updated after a run.
type ExpertStatsUpdate struct {
	RunID      string `json:"runId"`
	AgentID    string `json:"agentId"`
	StepID     string `json:"stepId"`
	TokensUsed int64  `json:"tokensUsed"`
	DurationMs int64  `json:"durationMs"`
	Status     string `json:"status"`
}

// LiveMetrics represents real-time platform metrics from the engine.
type LiveMetrics struct {
	ActiveAgents          int            `json:"activeAgents"`
	ActiveRuns            int            `json:"activeRuns"`
	TotalRuns             int            `json:"totalRuns"`
	TasksCompleted        int            `json:"tasksCompleted"`
	TasksFailed           int            `json:"tasksFailed"`
	TokensUsed            int64          `json:"tokensUsed"`
	AvgLatencyMs          int64          `json:"avgLatencyMs"`
	AvgTokensPerRun       int64          `json:"avgTokensPerRun"`
	SuccessRate           float64        `json:"successRate"`
	ActiveModels          map[string]int `json:"activeModels"`
	TotalActiveInferences int            `json:"totalActiveInferences"`
	MaxConcurrentAgents   int            `json:"maxConcurrentAgents"`
}

// RerunRequest re-runs a workflow step with a different model for comparison.
type RerunRequest struct {
	RunID       string  `json:"run_id"`
	StepID      string  `json:"step_id"`
	Engine      string  `json:"engine"`
	Model       string  `json:"model"`
	Temperature float64 `json:"temperature,omitempty"`
	MaxTokens   int     `json:"max_tokens,omitempty"`
}

// RerunResult is the response from a model comparison re-run.
type RerunResult struct {
	Text               string `json:"text"`
	TokensUsed         int64  `json:"tokensUsed"`
	DurationMs         int64  `json:"durationMs"`
	Model              string `json:"model"`
	Engine             string `json:"engine"`
	OriginalModel      string `json:"originalModel"`
	OriginalTokens     int64  `json:"originalTokens"`
	OriginalDurationMs int64  `json:"originalDurationMs"`
}

// AuditOperation represents a single operation in the execution audit trail.
type AuditOperation struct {
	ID         string         `json:"id"`
	RunID      string         `json:"run_id"`
	AgentID    string         `json:"agent_id"`
	Phase      string         `json:"phase"`
	Operation  string         `json:"operation"`
	Prompt     string         `json:"prompt,omitempty"`
	Response   string         `json:"response,omitempty"`
	TokensUsed int64          `json:"tokens_used"`
	DurationMs int64          `json:"duration_ms"`
	Status     string         `json:"status"`
	Error      string         `json:"error,omitempty"`
	Metadata   map[string]any `json:"metadata,omitempty"`
	CreatedAt  string         `json:"created_at"`
}

// ── Graph ──────────────────────────────────────────

// SimilarityEdge represents a weighted connection between two agents
// in the similarity graph.
type SimilarityEdge struct {
	Source string  `json:"source"`
	Target string  `json:"target"`
	Weight float64 `json:"weight"`
}

// GraphEdgesResponse is returned by the agent graph edges endpoint.
type GraphEdgesResponse struct {
	Edges   []SimilarityEdge `json:"edges"`
	Total   int              `json:"total"`
	Version string           `json:"version,omitempty"`
}

// EmbedAssetsRequest triggers re-embedding of an agent with file content.
type EmbedAssetsRequest struct {
	FileTexts []string `json:"file_texts"`
}

// EmbedBulkRequest sends a batch of expert definitions for embedding.
type EmbedBulkRequest struct {
	Experts []map[string]interface{} `json:"experts"`
	Source  string                   `json:"source"`
}
