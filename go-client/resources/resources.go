// Package resources provides resource management for kortecx parallel runs,
// including tasks, workflows, agents, and monitoring.
package resources

import (
	"fmt"
	"net/http"

	"github.com/exi/kortecx-go/client"
	"github.com/exi/kortecx-go/types"
)

// Service handles resource management for parallel runs.
type Service struct {
	c *client.Client
}

// New creates a new resources service.
func New(c *client.Client) *Service {
	return &Service{c: c}
}

// --- Tasks ---

// ListTasks returns all tasks in the queue. Pass optional ListOptions for pagination and sorting.
func (s *Service) ListTasks(opts ...*types.ListOptions) ([]types.Task, error) {
	path := "/api/tasks"
	if len(opts) > 0 {
		path = types.AppendQuery(path, opts[0])
	}
	var out []types.Task
	err := s.c.Do(http.MethodGet, path, nil, &out)
	return out, err
}

// CreateTask creates a new task.
func (s *Service) CreateTask(req types.CreateTaskRequest) (*types.Task, error) {
	var out types.Task
	err := s.c.Do(http.MethodPost, "/api/tasks", req, &out)
	return &out, err
}

// UpdateTask updates an existing task by ID.
func (s *Service) UpdateTask(id string, req types.UpdateTaskRequest) (*types.Task, error) {
	var out types.Task
	err := s.c.Do(http.MethodPatch, fmt.Sprintf("/api/tasks?id=%s", id), req, &out)
	return &out, err
}

// GetTask returns a task by ID.
func (s *Service) GetTask(id string) (*types.Task, error) {
	var out types.Task
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/tasks?id=%s", id), nil, &out)
	return &out, err
}

// ListTasksByExpert returns all tasks linked to a specific expert.
func (s *Service) ListTasksByExpert(expertID string) ([]types.Task, error) {
	var out []types.Task
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/tasks?expertId=%s", expertID), nil, &out)
	return out, err
}

// DeleteTask removes a task by ID.
func (s *Service) DeleteTask(id string) error {
	return s.c.Do(http.MethodDelete, fmt.Sprintf("/api/tasks?id=%s", id), nil, nil)
}

// --- Workflows ---

// ListWorkflows returns all workflows. Pass optional ListOptions for pagination and sorting.
func (s *Service) ListWorkflows(opts ...*types.ListOptions) ([]types.Workflow, error) {
	path := "/api/workflows"
	if len(opts) > 0 {
		path = types.AppendQuery(path, opts[0])
	}
	var out []types.Workflow
	err := s.c.Do(http.MethodGet, path, nil, &out)
	return out, err
}

// CreateWorkflow creates a new workflow.
func (s *Service) CreateWorkflow(req types.CreateWorkflowRequest) (*types.Workflow, error) {
	var out types.Workflow
	err := s.c.Do(http.MethodPost, "/api/workflows", req, &out)
	return &out, err
}

// GetWorkflow returns a workflow by ID including its steps.
func (s *Service) GetWorkflow(id string) (*types.Workflow, error) {
	var out types.Workflow
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/workflows?id=%s", id), nil, &out)
	return &out, err
}

// UpdateWorkflow updates an existing workflow.
func (s *Service) UpdateWorkflow(id string, updates map[string]any) (*types.Workflow, error) {
	updates["id"] = id
	var out types.Workflow
	err := s.c.Do(http.MethodPatch, "/api/workflows", updates, &out)
	return &out, err
}

// DeleteWorkflow removes a workflow and its steps by ID.
func (s *Service) DeleteWorkflow(id string) error {
	return s.c.Do(http.MethodDelete, fmt.Sprintf("/api/workflows?id=%s", id), nil, nil)
}

// RunWorkflow starts a workflow execution.
func (s *Service) RunWorkflow(workflowID string) (*types.WorkflowRun, error) {
	var out types.WorkflowRun
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/workflows/run?id=%s", workflowID), nil, &out)
	return &out, err
}

// ListRuns returns workflow execution history. Pass optional ListOptions for pagination and sorting.
func (s *Service) ListRuns(opts ...*types.ListOptions) ([]types.WorkflowRun, error) {
	path := "/api/workflows/runs"
	if len(opts) > 0 {
		path = types.AppendQuery(path, opts[0])
	}
	var out []types.WorkflowRun
	err := s.c.Do(http.MethodGet, path, nil, &out)
	return out, err
}

// GetWorkflowRun returns a specific workflow run by ID.
func (s *Service) GetWorkflowRun(id string) (*types.WorkflowRun, error) {
	var out types.WorkflowRun
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/workflows/runs?id=%s", id), nil, &out)
	return &out, err
}

// ExecuteWorkflow starts a full workflow execution with agent orchestration.
func (s *Service) ExecuteWorkflow(req types.WorkflowExecuteRequest) (*types.WorkflowRun, error) {
	var out types.WorkflowRun
	err := s.c.Do(http.MethodPost, "/api/orchestrator/execute", req, &out)
	return &out, err
}

// GetSharedMemory returns the shared memory snapshot for a workflow run.
func (s *Service) GetSharedMemory(runID string) (*types.SharedMemory, error) {
	var out types.SharedMemory
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/orchestrator/runs/%s/memory", runID), nil, &out)
	return &out, err
}

// --- Local Inference ---

// ListLocalModels returns available models on a local inference engine.
func (s *Service) ListLocalModels(engine string) ([]map[string]any, error) {
	var out struct {
		Engine string           `json:"engine"`
		Models []map[string]any `json:"models"`
	}
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/orchestrator/models/%s", engine), nil, &out)
	return out.Models, err
}

// CheckEngineHealth checks if a local inference engine is running.
func (s *Service) CheckEngineHealth(engine string) (bool, error) {
	var out struct {
		Healthy bool `json:"healthy"`
	}
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/orchestrator/health/%s", engine), nil, &out)
	return out.Healthy, err
}

// PullModel pulls/downloads a model on Ollama.
func (s *Service) PullModel(engine, model string) error {
	return s.c.Do(http.MethodPost, "/api/orchestrator/models/pull", map[string]string{
		"engine": engine,
		"model":  model,
	}, nil)
}

// LocalGenerate runs text generation on a local inference engine.
func (s *Service) LocalGenerate(req types.LocalGenerateRequest) (*types.LocalInferenceResult, error) {
	var out types.LocalInferenceResult
	err := s.c.Do(http.MethodPost, "/api/orchestrator/inference/generate", req, &out)
	return &out, err
}

// LocalChat runs chat completion on a local inference engine.
func (s *Service) LocalChat(req types.LocalChatRequest) (*types.LocalInferenceResult, error) {
	var out types.LocalInferenceResult
	err := s.c.Do(http.MethodPost, "/api/orchestrator/inference/chat", req, &out)
	return &out, err
}

// --- Model Comparison ---

// CompareModels runs a side-by-side comparison of two models via the engine.
func (s *Service) CompareModels(req types.CompareModelsRequest) (*types.CompareModelsResponse, error) {
	var out types.CompareModelsResponse
	err := s.c.Do(http.MethodPost, "/api/models/compare", req, &out)
	return &out, err
}

// --- Agents ---

// ListAgents returns all active agents and their task assignments. Pass optional ListOptions for pagination and sorting.
func (s *Service) ListAgents(opts ...*types.ListOptions) ([]types.Agent, error) {
	path := "/api/agents"
	if len(opts) > 0 {
		path = types.AppendQuery(path, opts[0])
	}
	var out []types.Agent
	err := s.c.Do(http.MethodGet, path, nil, &out)
	return out, err
}

// --- Monitoring ---

// GetMonitoring returns system monitoring snapshot (alerts, logs, latency, success rate).
func (s *Service) GetMonitoring() (*MonitoringSnapshot, error) {
	var out MonitoringSnapshot
	err := s.c.Do(http.MethodGet, "/api/monitoring", nil, &out)
	return &out, err
}

// ListAlerts returns system alerts. Pass optional ListOptions for pagination and sorting.
func (s *Service) ListAlerts(opts ...*types.ListOptions) ([]types.Alert, error) {
	path := "/api/alerts"
	if len(opts) > 0 {
		path = types.AppendQuery(path, opts[0])
	}
	var out []types.Alert
	err := s.c.Do(http.MethodGet, path, nil, &out)
	return out, err
}

// --- Assets ---

// ListAssets returns all assets, optionally filtered by the provided options.
func (s *Service) ListAssets(opts ...*types.AssetListOptions) ([]types.Asset, error) {
	path := "/api/assets"
	if len(opts) > 0 && opts[0] != nil {
		o := opts[0]
		q := "?"
		if o.Folder != "" {
			q += "folder=" + o.Folder + "&"
		}
		if o.ExpertID != "" {
			q += "expertId=" + o.ExpertID + "&"
		}
		if o.SourceType != "" {
			q += "sourceType=" + o.SourceType + "&"
		}
		if o.ExpertRunID != "" {
			q += "expertRunId=" + o.ExpertRunID + "&"
		}
		if o.Search != "" {
			q += "q=" + o.Search + "&"
		}
		if len(q) > 1 {
			path += q[:len(q)-1] // trim trailing &
		}
	}
	var resp struct {
		Assets []types.Asset `json:"assets"`
	}
	err := s.c.Do(http.MethodGet, path, nil, &resp)
	return resp.Assets, err
}

// GetAsset retrieves a single asset by ID.
func (s *Service) GetAsset(id string) (*types.Asset, error) {
	var resp struct {
		Assets []types.Asset `json:"assets"`
	}
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/assets?q=%s", id), nil, &resp)
	if err != nil {
		return nil, err
	}
	for i := range resp.Assets {
		if resp.Assets[i].ID == id {
			return &resp.Assets[i], nil
		}
	}
	return nil, fmt.Errorf("asset %s not found", id)
}

// DeleteAsset removes an asset record by ID.
func (s *Service) DeleteAsset(id string) error {
	return s.c.Do(http.MethodDelete, fmt.Sprintf("/api/assets?id=%s", id), nil, nil)
}

// RegisterAssets registers pre-existing files as asset records in the database.
func (s *Service) RegisterAssets(req types.RegisterAssetsRequest) ([]types.Asset, error) {
	var resp struct {
		Assets []types.Asset `json:"assets"`
	}
	err := s.c.Do(http.MethodPost, "/api/assets/register", req, &resp)
	return resp.Assets, err
}

// --- Expert Runs ---

// RunExpert starts a server-side expert execution that survives client disconnection.
func (s *Service) RunExpert(req types.RunExpertRequest) (string, error) {
	var resp struct {
		RunID  string `json:"runId"`
		Status string `json:"status"`
	}
	err := s.c.Do(http.MethodPost, "/api/experts/run", req, &resp)
	return resp.RunID, err
}

// GetExpertRun retrieves the status and result of an expert run.
func (s *Service) GetExpertRun(id string) (*types.ExpertRun, error) {
	var resp struct {
		Runs []types.ExpertRun `json:"runs"`
	}
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/experts/run?id=%s", id), nil, &resp)
	if err != nil {
		return nil, err
	}
	if len(resp.Runs) == 0 {
		return nil, fmt.Errorf("expert run %s not found", id)
	}
	return &resp.Runs[0], nil
}

// ListExpertRuns lists expert runs, optionally filtered by status.
func (s *Service) ListExpertRuns(status string) ([]types.ExpertRun, error) {
	path := "/api/experts/run"
	if status != "" {
		path += "?status=" + status
	}
	var resp struct {
		Runs []types.ExpertRun `json:"runs"`
	}
	err := s.c.Do(http.MethodGet, path, nil, &resp)
	return resp.Runs, err
}

// MonitoringSnapshot is the response from the monitoring endpoint.
type MonitoringSnapshot struct {
	Alerts      []types.Alert `json:"alerts"`
	Metrics     types.Metrics `json:"metrics"`
	RecentLogs  []LogEntry      `json:"recentLogs"`
}

// LogEntry is a single log from the kortecx system.
type LogEntry struct {
	ID        string         `json:"id"`
	Timestamp string         `json:"timestamp"`
	Level     string         `json:"level"`
	Message   string         `json:"message"`
	Source    string         `json:"source"`
	Metadata  map[string]any `json:"metadata,omitempty"`
	TaskID    string         `json:"taskId,omitempty"`
	RunID     string         `json:"runId,omitempty"`
}
