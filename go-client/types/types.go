package types

import "time"

// Expert roles
type ExpertRole string

const (
	RoleResearcher    ExpertRole = "researcher"
	RoleAnalyst       ExpertRole = "analyst"
	RoleWriter        ExpertRole = "writer"
	RoleCoder         ExpertRole = "coder"
	RoleReviewer      ExpertRole = "reviewer"
	RolePlanner       ExpertRole = "planner"
	RoleSynthesizer   ExpertRole = "synthesizer"
	RoleCritic        ExpertRole = "critic"
	RoleLegal         ExpertRole = "legal"
	RoleFinancial     ExpertRole = "financial"
	RoleMedical       ExpertRole = "medical"
	RoleCoordinator   ExpertRole = "coordinator"
	RoleDataEngineer  ExpertRole = "data-engineer"
	RoleCreative      ExpertRole = "creative"
	RoleTranslator    ExpertRole = "translator"
	RoleCustom        ExpertRole = "custom"
)

// Task status
type TaskStatus string

const (
	TaskQueued    TaskStatus = "queued"
	TaskRunning   TaskStatus = "running"
	TaskCompleted TaskStatus = "completed"
	TaskFailed    TaskStatus = "failed"
	TaskCancelled TaskStatus = "cancelled"
)

// Task priority
type TaskPriority string

const (
	PriorityCritical TaskPriority = "critical"
	PriorityHigh     TaskPriority = "high"
	PriorityNormal   TaskPriority = "normal"
	PriorityLow      TaskPriority = "low"
)

// Workflow status
type WorkflowStatus string

const (
	WorkflowDraft     WorkflowStatus = "draft"
	WorkflowReady     WorkflowStatus = "ready"
	WorkflowRunning   WorkflowStatus = "running"
	WorkflowPaused    WorkflowStatus = "paused"
	WorkflowCompleted WorkflowStatus = "completed"
	WorkflowFailed    WorkflowStatus = "failed"
	WorkflowCancelled WorkflowStatus = "cancelled"
)

// Dataset format
type DatasetFormat string

const (
	FormatJSONL   DatasetFormat = "jsonl"
	FormatCSV     DatasetFormat = "csv"
	FormatParquet DatasetFormat = "parquet"
)

// Training job status
type TrainingStatus string

const (
	TrainingQueued     TrainingStatus = "queued"
	TrainingPreparing  TrainingStatus = "preparing"
	TrainingRunning    TrainingStatus = "training"
	TrainingEvaluating TrainingStatus = "evaluating"
	TrainingCompleted  TrainingStatus = "completed"
	TrainingFailed     TrainingStatus = "failed"
)

// Alert severity
type AlertSeverity string

const (
	SeverityInfo     AlertSeverity = "info"
	SeverityWarning  AlertSeverity = "warning"
	SeverityError    AlertSeverity = "error"
	SeverityCritical AlertSeverity = "critical"
)

// ProviderID identifies a supported LLM provider.
type ProviderID string

const (
	ProviderAnthropic   ProviderID = "anthropic"
	ProviderOpenAI      ProviderID = "openai"
	ProviderGoogle      ProviderID = "google"
	ProviderHuggingFace ProviderID = "huggingface"
	ProviderDeepSeek    ProviderID = "deepseek"
	ProviderXAI         ProviderID = "xai"
)

// --- Core domain types ---

type Expert struct {
	ID               string            `json:"id"`
	Name             string            `json:"name"`
	Role             ExpertRole        `json:"role"`
	Status           string            `json:"status"`
	ModelID          string            `json:"modelId"`
	ModelName        string            `json:"modelName"`
	ProviderID       string            `json:"providerId"`
	ProviderName     string            `json:"providerName"`
	ModelSource      ModelSource       `json:"modelSource"`
	LocalModelConfig *LocalModelConfig `json:"localModelConfig,omitempty"`
	SystemPrompt     string            `json:"systemPrompt"`
	Temperature      float64           `json:"temperature"`
	MaxTokens        int               `json:"maxTokens"`
	IsFinetuned      bool              `json:"isFinetuned"`
	ReplicaCount     int               `json:"replicaCount"`
	TotalRuns        int               `json:"totalRuns"`
	SuccessRate      float64           `json:"successRate"`
	AvgLatencyMs     float64           `json:"avgLatencyMs"`
	Rating           float64           `json:"rating"`
	Tags             []string          `json:"tags,omitempty"`
	IsPublic         bool              `json:"isPublic"`
	Description      string            `json:"description"`
	CreatedAt        time.Time         `json:"createdAt"`
	UpdatedAt        time.Time         `json:"updatedAt"`
}

type Task struct {
	ID              string       `json:"id"`
	Name            string       `json:"name"`
	WorkflowID      string       `json:"workflowId,omitempty"`
	Status          TaskStatus   `json:"status"`
	Priority        TaskPriority `json:"priority"`
	CurrentStep     int          `json:"currentStep"`
	TotalSteps      int          `json:"totalSteps"`
	CurrentExpert   string       `json:"currentExpert,omitempty"`
	TokensUsed      int          `json:"tokensUsed"`
	EstimatedTokens int          `json:"estimatedTokens"`
	Progress        int          `json:"progress"`
	Input           string       `json:"input,omitempty"`
	Output          string       `json:"output,omitempty"`
	ErrorMessage    string       `json:"errorMessage,omitempty"`
	StartedAt       *time.Time   `json:"startedAt,omitempty"`
	CompletedAt     *time.Time   `json:"completedAt,omitempty"`
	CreatedAt       time.Time    `json:"createdAt"`
	UpdatedAt       time.Time    `json:"updatedAt"`
}

type Workflow struct {
	ID                   string         `json:"id"`
	Name                 string         `json:"name"`
	Description          string         `json:"description"`
	GoalStatement        string         `json:"goalStatement"`
	Status               WorkflowStatus `json:"status"`
	EstimatedTokens      int            `json:"estimatedTokens"`
	EstimatedCostUsd     float64        `json:"estimatedCostUsd"`
	EstimatedDurationSec int            `json:"estimatedDurationSec"`
	TotalRuns            int            `json:"totalRuns"`
	SuccessfulRuns       int            `json:"successfulRuns"`
	IsTemplate           bool           `json:"isTemplate"`
	TemplateCategory     string         `json:"templateCategory,omitempty"`
	Tags                 []string       `json:"tags"`
	CreatedAt            time.Time      `json:"createdAt"`
	UpdatedAt            time.Time      `json:"updatedAt"`
}

type WorkflowRun struct {
	ID              string         `json:"id"`
	WorkflowID      string         `json:"workflowId"`
	WorkflowName    string         `json:"workflowName"`
	Status          WorkflowStatus `json:"status"`
	StartedAt       time.Time      `json:"startedAt"`
	CompletedAt     *time.Time     `json:"completedAt,omitempty"`
	DurationSec     int            `json:"durationSec"`
	TotalTokensUsed int            `json:"totalTokensUsed"`
	TotalCostUsd    float64        `json:"totalCostUsd"`
	ExpertChain     []string       `json:"expertChain"`
	Metadata        map[string]any `json:"metadata,omitempty"`
}

type Dataset struct {
	ID           string        `json:"id"`
	Name         string        `json:"name"`
	Description  string        `json:"description"`
	Status       string        `json:"status"`
	Format       DatasetFormat `json:"format"`
	SampleCount  int           `json:"sampleCount"`
	SizeBytes    int64         `json:"sizeBytes"`
	QualityScore float64       `json:"qualityScore"`
	Tags         []string      `json:"tags"`
	Categories   []string      `json:"categories"`
	CreatedAt    time.Time     `json:"createdAt"`
	UpdatedAt    time.Time     `json:"updatedAt"`
}

type TrainingJob struct {
	ID                string         `json:"id"`
	Name              string         `json:"name"`
	ExpertID          string         `json:"expertId"`
	BaseModelID       string         `json:"baseModelId"`
	DatasetID         string         `json:"datasetId"`
	Status            TrainingStatus `json:"status"`
	Progress          int            `json:"progress"`
	Epochs            int            `json:"epochs"`
	CurrentEpoch      int            `json:"currentEpoch"`
	LearningRate      float64        `json:"learningRate"`
	BatchSize         int            `json:"batchSize"`
	TrainingSamples   int            `json:"trainingSamples"`
	ValidationSamples int            `json:"validationSamples"`
	EvalLoss          float64        `json:"evalLoss"`
	EvalAccuracy      float64        `json:"evalAccuracy"`
	GpuHours          float64        `json:"gpuHours"`
	CostUsd           float64        `json:"costUsd"`
	CreatedAt         time.Time      `json:"createdAt"`
	UpdatedAt         time.Time      `json:"updatedAt"`
}

type Provider struct {
	ID       string `json:"id"`
	Name     string `json:"name"`
	Type     string `json:"type"`
	Status   string `json:"status"`
	BaseURL  string `json:"baseUrl,omitempty"`
	APIKey   string `json:"apiKey,omitempty"`
	Models   []Model `json:"models,omitempty"`
}

type Model struct {
	ID           string   `json:"id"`
	Name         string   `json:"name"`
	ProviderID   string   `json:"providerId"`
	Capabilities []string `json:"capabilities"`
	CostPer1kIn  float64  `json:"costPer1kIn"`
	CostPer1kOut float64  `json:"costPer1kOut"`
	MaxContext   int      `json:"maxContext"`
}

type Agent struct {
	ID         string `json:"id"`
	ExpertID   string `json:"expertId"`
	ExpertName string `json:"expertName"`
	Status     string `json:"status"`
	TaskID     string `json:"taskId,omitempty"`
	TaskName   string `json:"taskName,omitempty"`
}

type Alert struct {
	ID             string        `json:"id"`
	Severity       AlertSeverity `json:"severity"`
	Title          string        `json:"title"`
	Message        string        `json:"message"`
	ProviderID     string        `json:"providerId,omitempty"`
	ExpertID       string        `json:"expertId,omitempty"`
	Acknowledged   bool          `json:"acknowledged"`
	AcknowledgedAt *time.Time    `json:"acknowledgedAt,omitempty"`
	ResolvedAt     *time.Time    `json:"resolvedAt,omitempty"`
	CreatedAt      time.Time     `json:"createdAt"`
}

type Metrics struct {
	ActiveAgents   int     `json:"activeAgents"`
	TasksCompleted int     `json:"tasksCompleted"`
	TokensUsed     int     `json:"tokensUsed"`
	AvgLatencyMs   float64 `json:"avgLatencyMs"`
	SuccessRate    float64 `json:"successRate"`
	CostUsd        float64 `json:"costUsd"`
	ErrorCount     int     `json:"errorCount"`
}

type Analytics struct {
	DailyBreakdown    []DailyMetric      `json:"dailyBreakdown"`
	ExpertPerformance []ExpertPerformance `json:"expertPerformance"`
	ProviderUsage     []ProviderUsage     `json:"providerUsage"`
}

type DailyMetric struct {
	Date           string  `json:"date"`
	TasksCompleted int     `json:"tasksCompleted"`
	TokensUsed     int     `json:"tokensUsed"`
	CostUsd        float64 `json:"costUsd"`
	SuccessRate    float64 `json:"successRate"`
}

type ExpertPerformance struct {
	ExpertID    string  `json:"expertId"`
	ExpertName  string  `json:"expertName"`
	TotalRuns   int     `json:"totalRuns"`
	SuccessRate float64 `json:"successRate"`
	AvgLatency  float64 `json:"avgLatency"`
}

type ProviderUsage struct {
	ProviderID   string  `json:"providerId"`
	ProviderName string  `json:"providerName"`
	Percentage   float64 `json:"percentage"`
	TotalTokens  int     `json:"totalTokens"`
}

// --- Assets ---

// Asset represents a file or artifact stored on disk with DB metadata.
type Asset struct {
	ID          string         `json:"id"`
	Name        string         `json:"name"`
	Description string         `json:"description,omitempty"`
	Folder      string         `json:"folder"`
	MimeType    string         `json:"mimeType"`
	FileType    string         `json:"fileType"`
	FilePath    string         `json:"filePath"`
	FileName    string         `json:"fileName"`
	SizeBytes   int64          `json:"sizeBytes"`
	Tags        []string       `json:"tags,omitempty"`
	Metadata    map[string]any `json:"metadata,omitempty"`
	ExpertID    string         `json:"expertId,omitempty"`
	ExpertRunID string         `json:"expertRunId,omitempty"`
	SourceType  string         `json:"sourceType,omitempty"`
	DatasetID   string         `json:"datasetId,omitempty"`
	CreatedAt   time.Time      `json:"createdAt"`
	UpdatedAt   time.Time      `json:"updatedAt"`
}

// AssetListOptions provides filter parameters for listing assets.
type AssetListOptions struct {
	Folder      string `json:"folder,omitempty"`
	ExpertID    string `json:"expertId,omitempty"`
	SourceType  string `json:"sourceType,omitempty"`
	ExpertRunID string `json:"expertRunId,omitempty"`
	Search      string `json:"q,omitempty"`
}

// RegisterAssetsRequest is the request body for bulk asset registration.
type RegisterAssetsRequest struct {
	Assets []Asset `json:"assets"`
}

// --- Expert Runs ---

// ExpertRun tracks a single expert execution with its result.
type ExpertRun struct {
	ID            string     `json:"id"`
	ExpertID      string     `json:"expertId"`
	ExpertName    string     `json:"expertName"`
	Status        string     `json:"status"`
	Model         string     `json:"model,omitempty"`
	Engine        string     `json:"engine,omitempty"`
	Temperature   float64    `json:"temperature,omitempty"`
	MaxTokens     int        `json:"maxTokens,omitempty"`
	SystemPrompt  string     `json:"systemPrompt,omitempty"`
	UserPrompt    string     `json:"userPrompt,omitempty"`
	ResponseText  string     `json:"responseText,omitempty"`
	TokensUsed    int        `json:"tokensUsed"`
	DurationMs    int        `json:"durationMs"`
	ArtifactCount int        `json:"artifactCount"`
	ErrorMessage  string     `json:"errorMessage,omitempty"`
	Metadata      map[string]any `json:"metadata,omitempty"`
	StartedAt     *time.Time `json:"startedAt,omitempty"`
	CompletedAt   *time.Time `json:"completedAt,omitempty"`
	CreatedAt     time.Time  `json:"createdAt"`
}

// RunExpertRequest starts a server-side expert execution.
type RunExpertRequest struct {
	ExpertID     string   `json:"expertId"`
	ExpertName   string   `json:"expertName"`
	Model        string   `json:"model,omitempty"`
	Engine       string   `json:"engine,omitempty"`
	Temperature  float64  `json:"temperature,omitempty"`
	MaxTokens    int      `json:"maxTokens,omitempty"`
	SystemPrompt string   `json:"systemPrompt,omitempty"`
	UserPrompt   string   `json:"userPrompt,omitempty"`
	Role         string   `json:"role,omitempty"`
	Tags         []string `json:"tags,omitempty"`
}

// --- Model Source ---

// ModelSource indicates whether an expert runs on local inference or a cloud provider.
type ModelSource string

const (
	ModelSourceLocal    ModelSource = "local"
	ModelSourceProvider ModelSource = "provider"
)

// LocalInferenceEngine identifies which local inference server to use.
type LocalInferenceEngine string

const (
	EngineOllama   LocalInferenceEngine = "ollama"
	EngineLlamaCpp LocalInferenceEngine = "llamacpp"
	EngineVLLM     LocalInferenceEngine = "vllm"
)

// LocalModelConfig configures a locally-hosted model.
type LocalModelConfig struct {
	Engine  LocalInferenceEngine `json:"engine"`
	Model   string               `json:"model"`
	BaseURL string               `json:"baseUrl,omitempty"`
}

// --- Agent Memory & Orchestration ---

// AgentMemory holds an agent's private execution state.
type AgentMemory struct {
	Plan     string   `json:"plan"`
	Context  string   `json:"context"`
	Findings []string `json:"findings"`
}

// SharedMemory is the run-level shared memory accessible by all agents.
type SharedMemory struct {
	RunID   string            `json:"runId"`
	Entries map[string]string `json:"entries"` // agentId -> serialized memory
	Globals map[string]string `json:"globals"` // shared key-value store
}

// AgentEvent is a real-time event emitted during workflow execution.
type AgentEvent struct {
	RunID   string `json:"runId"`
	AgentID string `json:"agentId"`
	StepID  string `json:"stepId"`
	Event   string `json:"event"`
	Data    any    `json:"data"`
}

// --- Workflow Execution ---

// WorkflowExecuteRequest initiates a full workflow run with agents.
type WorkflowExecuteRequest struct {
	WorkflowID    string               `json:"workflowId,omitempty"`
	Name          string               `json:"name"`
	GoalFileURL   string               `json:"goalFileUrl"`
	InputFileURLs []string             `json:"inputFileUrls"`
	Steps         []WorkflowStepConfig `json:"steps"`
}

// WorkflowStepConfig defines a single step's configuration for execution.
type WorkflowStepConfig struct {
	StepID          string            `json:"stepId"`
	ExpertID        string            `json:"expertId,omitempty"`
	TaskDescription string            `json:"taskDescription"`
	ModelSource     ModelSource       `json:"modelSource"`
	LocalModel      *LocalModelConfig `json:"localModel,omitempty"`
	Temperature     float64           `json:"temperature"`
	MaxTokens       int               `json:"maxTokens"`
	ConnectionType  string            `json:"connectionType"` // sequential | parallel
}

// --- Integrations & Plugins ---

// IntegrationCategory classifies external integrations.
type IntegrationCategory string

const (
	IntegrationAPI           IntegrationCategory = "api"
	IntegrationApp           IntegrationCategory = "app"
	IntegrationTool          IntegrationCategory = "tool"
	IntegrationDatabase      IntegrationCategory = "database"
	IntegrationStorage       IntegrationCategory = "storage"
	IntegrationMessaging     IntegrationCategory = "messaging"
	IntegrationAnalytics     IntegrationCategory = "analytics"
	IntegrationSocial        IntegrationCategory = "social"
	IntegrationCRM           IntegrationCategory = "crm"
	IntegrationDataAnalytics IntegrationCategory = "data_analytics"
)

// IntegrationCapability describes what an integration can do in the agentic lifecycle.
type IntegrationCapability string

const (
	CapabilityConsume  IntegrationCapability = "consume"
	CapabilityGenerate IntegrationCapability = "generate"
	CapabilityPublish  IntegrationCapability = "publish"
	CapabilitySchedule IntegrationCapability = "schedule"
	CapabilityReport   IntegrationCapability = "report"
	CapabilityExecute  IntegrationCapability = "execute"
)

// Integration represents an external service connection.
type Integration struct {
	ID           string                  `json:"id"`
	Name         string                  `json:"name"`
	Description  string                  `json:"description"`
	Category     IntegrationCategory     `json:"category"`
	Icon         string                  `json:"icon"`
	Color        string                  `json:"color"`
	Connected    bool                    `json:"connected"`
	AuthType     string                  `json:"authType"`
	Capabilities []IntegrationCapability `json:"capabilities"`
	BaseURL      string                  `json:"baseUrl,omitempty"`
	DocsURL      string                  `json:"docsUrl,omitempty"`
	CreatedAt    time.Time               `json:"createdAt"`
	UpdatedAt    time.Time               `json:"updatedAt"`
}

// IntegrationConnection is a user's configured instance of an integration.
type IntegrationConnection struct {
	ID            string            `json:"id"`
	IntegrationID string            `json:"integrationId"`
	Name          string            `json:"name"`
	Config        map[string]string `json:"config"`
	Status        string            `json:"status"` // active | error | expired
	LastTestedAt  *time.Time        `json:"lastTestedAt,omitempty"`
	CreatedAt     time.Time         `json:"createdAt"`
}

// PluginSource indicates where a plugin comes from.
type PluginSource string

const (
	PluginPersonal    PluginSource = "personal"
	PluginMarketplace PluginSource = "marketplace"
)

// Plugin represents an installable extension for workflow agents.
type Plugin struct {
	ID           string       `json:"id"`
	Name         string       `json:"name"`
	Description  string       `json:"description"`
	Version      string       `json:"version"`
	Author       string       `json:"author"`
	Source       PluginSource `json:"source"`
	Status       string       `json:"status"` // active | disabled | error | installing
	Icon         string       `json:"icon"`
	Color        string       `json:"color"`
	Category     string       `json:"category"`
	Capabilities []string     `json:"capabilities"`
	Installed    bool         `json:"installed"`
	Downloads    int          `json:"downloads,omitempty"`
	Rating       float64      `json:"rating,omitempty"`
	CreatedAt    time.Time    `json:"createdAt"`
	UpdatedAt    time.Time    `json:"updatedAt"`
}

// StepIntegration is an integration or plugin attached to a workflow step.
type StepIntegration struct {
	ID          string            `json:"id"`
	Type        string            `json:"type"`        // integration | plugin
	ReferenceID string            `json:"referenceId"` // integration or plugin ID
	Name        string            `json:"name"`
	Icon        string            `json:"icon"`
	Color       string            `json:"color"`
	Config      map[string]string `json:"config,omitempty"`
}

// --- Local Inference Request/Response ---

// LocalGenerateRequest sends a text generation request to a local engine.
type LocalGenerateRequest struct {
	Engine      string  `json:"engine"`
	Model       string  `json:"model"`
	Prompt      string  `json:"prompt"`
	System      string  `json:"system,omitempty"`
	Temperature float64 `json:"temperature"`
	MaxTokens   int     `json:"maxTokens"`
	BaseURL     string  `json:"baseUrl,omitempty"`
}

// LocalChatRequest sends a chat request to a local engine.
type LocalChatRequest struct {
	Engine      string              `json:"engine"`
	Model       string              `json:"model"`
	Messages    []map[string]string `json:"messages"`
	Temperature float64             `json:"temperature"`
	MaxTokens   int                 `json:"maxTokens"`
	BaseURL     string              `json:"baseUrl,omitempty"`
}

// LocalInferenceResult is the response from local generate/chat.
type LocalInferenceResult struct {
	Text       string  `json:"text"`
	TokensUsed int     `json:"tokensUsed"`
	Model      string  `json:"model"`
	DurationMs float64 `json:"durationMs"`
	Error      string  `json:"error,omitempty"`
}

// --- Pagination ---

// ListOptions provides optional pagination and sorting for list operations.
type ListOptions struct {
	Limit  int    `json:"limit,omitempty"`
	Offset int    `json:"offset,omitempty"`
	Sort   string `json:"sort,omitempty"`
}

// --- Request types ---

type CreateTaskRequest struct {
	Name            string       `json:"name"`
	WorkflowID      string       `json:"workflowId,omitempty"`
	Priority        TaskPriority `json:"priority"`
	Input           string       `json:"input"`
	EstimatedTokens int          `json:"estimatedTokens,omitempty"`
}

type UpdateTaskRequest struct {
	Status       *TaskStatus `json:"status,omitempty"`
	Progress     *int        `json:"progress,omitempty"`
	Output       *string     `json:"output,omitempty"`
	ErrorMessage *string     `json:"errorMessage,omitempty"`
}

type CreateWorkflowRequest struct {
	Name          string   `json:"name"`
	Description   string   `json:"description"`
	GoalStatement string   `json:"goalStatement"`
	Tags          []string `json:"tags,omitempty"`
}

type DeployExpertRequest struct {
	Name             string            `json:"name"`
	Role             ExpertRole        `json:"role"`
	Description      string            `json:"description,omitempty"`
	ModelID          string            `json:"modelId"`
	ProviderID       string            `json:"providerId"`
	ModelSource      ModelSource       `json:"modelSource"`
	LocalModelConfig *LocalModelConfig `json:"localModelConfig,omitempty"`
	SystemPrompt     string            `json:"systemPrompt"`
	Temperature      float64           `json:"temperature"`
	MaxTokens        int               `json:"maxTokens"`
	Tags             []string          `json:"tags,omitempty"`
	IsPublic         bool              `json:"isPublic"`
	ReplicaCount     int               `json:"replicaCount,omitempty"`
}

// UpdateExpertRequest updates an existing expert's configuration.
type UpdateExpertRequest struct {
	Name             *string           `json:"name,omitempty"`
	Description      *string           `json:"description,omitempty"`
	Role             *ExpertRole       `json:"role,omitempty"`
	Status           *string           `json:"status,omitempty"`
	ModelID          *string           `json:"modelId,omitempty"`
	ProviderID       *string           `json:"providerId,omitempty"`
	ModelSource      *ModelSource      `json:"modelSource,omitempty"`
	LocalModelConfig *LocalModelConfig `json:"localModelConfig,omitempty"`
	SystemPrompt     *string           `json:"systemPrompt,omitempty"`
	Temperature      *float64          `json:"temperature,omitempty"`
	MaxTokens        *int              `json:"maxTokens,omitempty"`
	Tags             []string          `json:"tags,omitempty"`
	IsPublic         *bool             `json:"isPublic,omitempty"`
}

type CreateDatasetRequest struct {
	Name        string        `json:"name"`
	Description string        `json:"description"`
	Format      DatasetFormat `json:"format"`
	Tags        []string      `json:"tags,omitempty"`
	Categories  []string      `json:"categories,omitempty"`
}

type StartTrainingRequest struct {
	Name         string  `json:"name"`
	ExpertID     string  `json:"expertId"`
	BaseModelID  string  `json:"baseModelId"`
	DatasetID    string  `json:"datasetId"`
	Epochs       int     `json:"epochs"`
	LearningRate float64 `json:"learningRate"`
	BatchSize    int     `json:"batchSize"`
}

// UpdateProviderRequest updates an existing provider's configuration.
type UpdateProviderRequest struct {
	Name    *string `json:"name,omitempty"`
	BaseURL *string `json:"baseUrl,omitempty"`
	APIKey  *string `json:"apiKey,omitempty"`
}

// UpdateDatasetRequest updates an existing dataset.
type UpdateDatasetRequest struct {
	Name        *string  `json:"name,omitempty"`
	Description *string  `json:"description,omitempty"`
	Status      *string  `json:"status,omitempty"`
	Tags        []string `json:"tags,omitempty"`
}

type CreateProviderRequest struct {
	Name    string `json:"name"`
	Type    string `json:"type"`
	BaseURL string `json:"baseUrl"`
	APIKey  string `json:"apiKey"`
}

/* ── MCP Server Types ─────────────────────────────────────────────────── */

// McpServerStatus represents the status of an MCP server script.
type McpServerStatus string

const (
	McpStatusIdle    McpServerStatus = "idle"
	McpStatusRunning McpServerStatus = "running"
	McpStatusTested  McpServerStatus = "tested"
	McpStatusError   McpServerStatus = "error"
)

// McpServerSource represents where an MCP server script originates from.
type McpServerSource string

const (
	McpSourcePrebuilt  McpServerSource = "prebuilt"
	McpSourceGenerated McpServerSource = "generated"
	McpSourcePersisted McpServerSource = "persisted"
)

// McpServer represents an MCP server script.
type McpServer struct {
	ID          string          `json:"id"`
	Name        string          `json:"name"`
	Description string          `json:"description"`
	Language    string          `json:"language"`   // python | typescript | javascript
	Filename    string          `json:"filename"`
	Source      McpServerSource `json:"source"`
	Code        string          `json:"code"`
	Status      McpServerStatus `json:"status"`
	TestOutput       string          `json:"test_output,omitempty"`
	CreatedAt        string          `json:"created_at,omitempty"`
	Prompt           string          `json:"prompt,omitempty"`
	IsPublic         bool            `json:"is_public"`
	GenerationTimeMs int             `json:"generation_time_ms,omitempty"`
	CpuPercent       float64         `json:"cpu_percent,omitempty"`
}

// McpServersResponse is the response from GET /api/mcp/servers.
type McpServersResponse struct {
	Prebuilt  []McpServer `json:"prebuilt"`
	Persisted []McpServer `json:"persisted"`
	Cached    []McpServer `json:"cached"`
	Total     int         `json:"total"`
}

// GenerateMcpRequest is the request to POST /api/mcp/generate.
type GenerateMcpRequest struct {
	Prompt   string `json:"prompt"`
	Language string `json:"language"` // python | typescript | javascript
	Model    string `json:"model"`
	Source   string `json:"source"` // ollama | llamacpp
}

// CacheMcpRequest is the request to POST /api/mcp/cache.
type CacheMcpRequest struct {
	Name        string `json:"name"`
	Description string `json:"description"`
	Language    string `json:"language"`
	Code        string `json:"code"`
	Filename    string `json:"filename,omitempty"`
}

// --- Model Comparison ---

// CompareModelsRequest sends a side-by-side model comparison request.
type CompareModelsRequest struct {
	Prompt       string   `json:"prompt"`
	SystemPrompt string   `json:"system_prompt,omitempty"`
	ModelA       string   `json:"model_a"`
	EngineA      string   `json:"engine_a"`
	ModelB       string   `json:"model_b"`
	EngineB      string   `json:"engine_b"`
	Temperature  float64  `json:"temperature"`
	MaxTokens    int      `json:"max_tokens"`
	DocumentURLs []string `json:"document_urls,omitempty"`
}

// CompareModelResult is the result for a single model in a comparison.
type CompareModelResult struct {
	Model        string  `json:"model"`
	Engine       string  `json:"engine"`
	Response     string  `json:"response"`
	Tokens       int     `json:"tokens"`
	DurationMs   int     `json:"duration_ms"`
	TokensPerSec float64 `json:"tokens_per_sec"`
	Error        *string `json:"error,omitempty"`
}

// CompareModelsResponse is the response from POST /api/models/compare.
type CompareModelsResponse struct {
	ModelA        CompareModelResult `json:"model_a"`
	ModelB        CompareModelResult `json:"model_b"`
	Temperature   float64            `json:"temperature"`
	Prompt        string             `json:"prompt"`
	MLflowRunID   *string            `json:"mlflow_run_id"`
	DocumentCount int                `json:"document_count,omitempty"`
	DocumentNames []string           `json:"document_names,omitempty"`
}

// --- Integration Request/Response ---

// CreateConnectionRequest creates a new integration connection.
type CreateConnectionRequest struct {
	IntegrationID string            `json:"integrationId"`
	Name          string            `json:"name"`
	Config        map[string]string `json:"config"`
}

// ConnectionTestResult is the response from testing an integration connection.
type ConnectionTestResult struct {
	Success  bool   `json:"success"`
	Message  string `json:"message"`
	Latency  int    `json:"latencyMs,omitempty"`
	TestedAt string `json:"testedAt,omitempty"`
}
