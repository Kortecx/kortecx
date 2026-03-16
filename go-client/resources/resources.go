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

// ListTasks returns all tasks in the queue.
func (s *Service) ListTasks() ([]types.Task, error) {
	var out []types.Task
	err := s.c.Do(http.MethodGet, "/api/tasks", nil, &out)
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

// --- Workflows ---

// ListWorkflows returns all workflows.
func (s *Service) ListWorkflows() ([]types.Workflow, error) {
	var out []types.Workflow
	err := s.c.Do(http.MethodGet, "/api/workflows", nil, &out)
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

// ListRuns returns workflow execution history.
func (s *Service) ListRuns() ([]types.WorkflowRun, error) {
	var out []types.WorkflowRun
	err := s.c.Do(http.MethodGet, "/api/workflows/runs", nil, &out)
	return out, err
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

// --- Agents ---

// ListAgents returns all active agents and their task assignments.
func (s *Service) ListAgents() ([]types.Agent, error) {
	var out []types.Agent
	err := s.c.Do(http.MethodGet, "/api/agents", nil, &out)
	return out, err
}

// --- Monitoring ---

// GetMonitoring returns system monitoring snapshot (alerts, logs, latency, success rate).
func (s *Service) GetMonitoring() (*MonitoringSnapshot, error) {
	var out MonitoringSnapshot
	err := s.c.Do(http.MethodGet, "/api/monitoring", nil, &out)
	return &out, err
}

// ListAlerts returns system alerts.
func (s *Service) ListAlerts() ([]types.Alert, error) {
	var out []types.Alert
	err := s.c.Do(http.MethodGet, "/api/alerts", nil, &out)
	return out, err
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
