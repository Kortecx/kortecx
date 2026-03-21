package quorum

import (
	"encoding/json"
	"fmt"

	"github.com/exi/kortecx-go/ws"
)

// Event constants for the quorum WebSocket protocol.
const (
	// Client -> Server (requests)
	EventSubmitRun      = "quorum.run.submit"
	EventCancelRun      = "quorum.run.cancel"
	EventRunStatus      = "quorum.run.status"
	EventGetRun         = "quorum.run.get"
	EventListRuns       = "quorum.run.list"
	EventDeleteRun      = "quorum.run.delete"
	EventGetConfig      = "quorum.config.get"
	EventUpdateConfig   = "quorum.config.update"
	EventListModels     = "quorum.models.list"
	EventPullModel      = "quorum.models.pull"
	EventGetMetrics     = "quorum.metrics.get"
	EventMetricsHistory = "quorum.metrics.history"
	EventListOperations = "quorum.operations.list"
	EventSubscribe      = "quorum.subscribe"
	EventSubscribeAll   = "quorum.subscribe.all"
	EventUnsubscribe    = "quorum.unsubscribe"

	// Server -> Client (push events)
	EventRunQueued      = "quorum.run.queued"
	EventRunStarted     = "quorum.run.started"
	EventRunComplete    = "quorum.run.complete"
	EventRunFailed      = "quorum.run.failed"
	EventPhaseUpdate    = "quorum.phase.update"
	EventAgentCreated   = "quorum.agent.created"
	EventAgentThinking  = "quorum.agent.thinking"
	EventAgentOutput    = "quorum.agent.output"
	EventAgentFailed    = "quorum.agent.failed"
	EventAgentRecovered = "quorum.agent.recovered"
	EventMetricsSnap    = "quorum.metrics.snapshot"
	EventQuorumError    = "quorum.error"

	// Expert workflow execution events
	EventExpertStatsUpdate = "expert.stats.update"
	EventWorkflowExecute   = "workflow.execute"
)

// Service provides the client-side API for the Quorum engine.
// It wraps a ws.Conn and exposes typed methods for every quorum operation.
type Service struct {
	ws *ws.Conn
}

// New creates a new Quorum client service bound to the given WebSocket connection.
func New(conn *ws.Conn) *Service {
	return &Service{ws: conn}
}

// ── Run Operations ────────────────────────────────────────

// SubmitRun sends a new quorum run request to the server.
func (s *Service) SubmitRun(req SubmitRequest) error {
	return s.send(EventSubmitRun, req)
}

// CancelRun requests cancellation of the specified run.
func (s *Service) CancelRun(runID string) error {
	return s.send(EventCancelRun, RunID{RunID: runID})
}

// GetRunStatus requests the current status of a run.
func (s *Service) GetRunStatus(runID string) error {
	return s.send(EventRunStatus, RunID{RunID: runID})
}

// GetRun requests full details for a single run.
func (s *Service) GetRun(runID string) error {
	return s.send(EventGetRun, RunID{RunID: runID})
}

// ListRuns requests a filtered list of runs.
func (s *Service) ListRuns(filter RunListFilter) error {
	return s.send(EventListRuns, filter)
}

// DeleteRun requests permanent deletion of a run and its artefacts.
func (s *Service) DeleteRun(runID string) error {
	return s.send(EventDeleteRun, RunID{RunID: runID})
}

// ── Config ────────────────────────────────────────────────

// GetConfig requests the quorum configuration for a project.
func (s *Service) GetConfig(project string) error {
	return s.send(EventGetConfig, ProjectConfig{Project: project})
}

// UpdateConfig sends an updated quorum configuration for a project.
func (s *Service) UpdateConfig(project string, config map[string]any) error {
	return s.send(EventUpdateConfig, ProjectConfig{Project: project, Config: config})
}

// ── Models ────────────────────────────────────────────────

// ListModels requests the available models for a given inference backend.
func (s *Service) ListModels(backend string) error {
	return s.send(EventListModels, ModelsRequest{Backend: backend})
}

// PullModel requests the server to download a model for the specified backend.
func (s *Service) PullModel(model, backend string) error {
	return s.send(EventPullModel, PullModelRequest{Model: model, Backend: backend})
}

// ── Metrics ───────────────────────────────────────────────

// GetMetrics requests a point-in-time metrics snapshot from the engine.
func (s *Service) GetMetrics() error {
	return s.send(EventGetMetrics, nil)
}

// GetMetricsHistory requests historical metrics snapshots (most recent first).
func (s *Service) GetMetricsHistory(limit int) error {
	return s.send(EventMetricsHistory, MetricsHistoryRequest{Limit: limit})
}

// ── Operations ────────────────────────────────────────────

// ListOperations requests low-level operation records matching the filter.
func (s *Service) ListOperations(filter OperationsFilter) error {
	return s.send(EventListOperations, filter)
}

// ── Subscriptions ─────────────────────────────────────────

// SubscribeRun subscribes to real-time events for a single run.
func (s *Service) SubscribeRun(runID string) error {
	return s.send(EventSubscribe, RunID{RunID: runID})
}

// SubscribeAll subscribes to real-time events for all active runs.
func (s *Service) SubscribeAll() error {
	return s.send(EventSubscribeAll, nil)
}

// UnsubscribeRun removes the subscription for a single run.
func (s *Service) UnsubscribeRun(runID string) error {
	return s.send(EventUnsubscribe, RunID{RunID: runID})
}

// ── Event Handlers ────────────────────────────────────────

// OnRunEvent registers a handler for all quorum run lifecycle events
// (queued, started, complete, failed).
func (s *Service) OnRunEvent(h ws.Handler) {
	s.ws.On(EventRunQueued, h)
	s.ws.On(EventRunStarted, h)
	s.ws.On(EventRunComplete, h)
	s.ws.On(EventRunFailed, h)
}

// OnPhaseEvent registers a handler for phase update events.
func (s *Service) OnPhaseEvent(h ws.Handler) {
	s.ws.On(EventPhaseUpdate, h)
}

// OnAgentEvent registers a handler for all quorum agent lifecycle events
// (created, thinking, output, failed, recovered).
func (s *Service) OnAgentEvent(h ws.Handler) {
	s.ws.On(EventAgentCreated, h)
	s.ws.On(EventAgentThinking, h)
	s.ws.On(EventAgentOutput, h)
	s.ws.On(EventAgentFailed, h)
	s.ws.On(EventAgentRecovered, h)
}

// OnMetrics registers a handler for periodic metrics snapshot events.
func (s *Service) OnMetrics(h ws.Handler) {
	s.ws.On(EventMetricsSnap, h)
}

// OnError registers a handler for quorum error events.
func (s *Service) OnError(h ws.Handler) {
	s.ws.On(EventQuorumError, h)
}

// ── Expert Workflow Execution ─────────────────────────

// ExecuteExpertWorkflow sends a workflow execution request with expert-based agents.
// This triggers the orchestrator on the engine side.
func (s *Service) ExecuteExpertWorkflow(req ExpertRunRequest) error {
	return s.send(EventWorkflowExecute, req)
}

// OnExpertStatsUpdate registers a handler for expert performance stat updates.
func (s *Service) OnExpertStatsUpdate(h ws.Handler) {
	s.ws.On(EventExpertStatsUpdate, h)
}

// OnAll registers a handler for every quorum server-push event.
func (s *Service) OnAll(h ws.Handler) {
	s.OnRunEvent(h)
	s.OnPhaseEvent(h)
	s.OnAgentEvent(h)
	s.OnMetrics(h)
	s.OnExpertStatsUpdate(h)
	s.OnError(h)
}

// ── Helpers ───────────────────────────────────────────────

func (s *Service) send(event string, payload any) error {
	if payload == nil {
		return s.ws.Send(ws.Message{Event: event})
	}
	data, err := json.Marshal(payload)
	if err != nil {
		return fmt.Errorf("marshaling %s payload: %w", event, err)
	}
	return s.ws.Send(ws.Message{Event: event, Data: data})
}

// ParseData unmarshals the Data field of a ws.Message into the target type.
func ParseData[T any](msg ws.Message) (T, error) {
	var out T
	if err := json.Unmarshal(msg.Data, &out); err != nil {
		return out, fmt.Errorf("parsing %s data: %w", msg.Event, err)
	}
	return out, nil
}
