package quorum

import (
	"log/slog"

	"github.com/exi/kortecx-go/ws"
)

// Callbacks holds typed callback functions for quorum server-push events.
// Nil callbacks are silently ignored during registration.
type Callbacks struct {
	OnRunQueued      func(data RunStatus)
	OnRunStarted     func(data RunStatus)
	OnRunComplete    func(data RunResult)
	OnRunFailed      func(data ErrorResponse)
	OnPhaseUpdate    func(data PhaseUpdate)
	OnAgentCreated   func(data AgentCreated)
	OnAgentThinking  func(data AgentThinking)
	OnAgentOutput    func(data AgentOutput)
	OnAgentFailed    func(data AgentFailed)
	OnAgentRecovered func(data AgentRecovered)
	OnMetrics           func(data MetricsSnapshot)
	OnError             func(data ErrorResponse)
	OnExpertStatsUpdate func(data ExpertStatsUpdate)
}

// RegisterCallbacks registers typed event handlers on the given quorum service.
// Each non-nil callback in cb is wired to the corresponding WebSocket event.
// Parse errors are logged and the callback is skipped for that message.
func RegisterCallbacks(svc *Service, cb Callbacks) {
	if cb.OnRunQueued != nil {
		svc.ws.On(EventRunQueued, makeHandler(cb.OnRunQueued))
	}
	if cb.OnRunStarted != nil {
		svc.ws.On(EventRunStarted, makeHandler(cb.OnRunStarted))
	}
	if cb.OnRunComplete != nil {
		svc.ws.On(EventRunComplete, makeHandler(cb.OnRunComplete))
	}
	if cb.OnRunFailed != nil {
		svc.ws.On(EventRunFailed, makeHandler(cb.OnRunFailed))
	}
	if cb.OnPhaseUpdate != nil {
		svc.ws.On(EventPhaseUpdate, makeHandler(cb.OnPhaseUpdate))
	}
	if cb.OnAgentCreated != nil {
		svc.ws.On(EventAgentCreated, makeHandler(cb.OnAgentCreated))
	}
	if cb.OnAgentThinking != nil {
		svc.ws.On(EventAgentThinking, makeHandler(cb.OnAgentThinking))
	}
	if cb.OnAgentOutput != nil {
		svc.ws.On(EventAgentOutput, makeHandler(cb.OnAgentOutput))
	}
	if cb.OnAgentFailed != nil {
		svc.ws.On(EventAgentFailed, makeHandler(cb.OnAgentFailed))
	}
	if cb.OnAgentRecovered != nil {
		svc.ws.On(EventAgentRecovered, makeHandler(cb.OnAgentRecovered))
	}
	if cb.OnMetrics != nil {
		svc.ws.On(EventMetricsSnap, makeHandler(cb.OnMetrics))
	}
	if cb.OnError != nil {
		svc.ws.On(EventQuorumError, makeHandler(cb.OnError))
	}
	if cb.OnExpertStatsUpdate != nil {
		svc.ws.On(EventExpertStatsUpdate, makeHandler(cb.OnExpertStatsUpdate))
	}
}

// makeHandler wraps a typed callback into a ws.Handler that deserialises the
// message data before invoking the callback. Parse failures are logged.
func makeHandler[T any](cb func(T)) ws.Handler {
	return func(msg ws.Message) {
		data, err := ParseData[T](msg)
		if err != nil {
			slog.Warn("quorum event parse failed", slog.String("component", "quorum"), slog.String("event", msg.Event), slog.String("error", err.Error()))
			return
		}
		cb(data)
	}
}
