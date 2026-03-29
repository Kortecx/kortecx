package ws

import (
	"encoding/json"
	"fmt"
	"log/slog"
	"sync"
	"time"

	"github.com/gorilla/websocket"
)

// Event types sent/received over the WebSocket.
const (
	EventTaskCreated    = "task.created"
	EventTaskUpdated    = "task.updated"
	EventTaskCompleted  = "task.completed"
	EventTaskFailed     = "task.failed"
	EventRunStarted     = "run.started"
	EventRunCompleted   = "run.completed"
	EventRunFailed      = "run.failed"
	EventAgentStatus    = "agent.status"
	EventExpertDeployed = "expert.deployed"
	EventAlert          = "alert"
	EventMetrics        = "metrics"
	EventPing           = "ping"
	EventPong           = "pong"
	EventSubscribe      = "subscribe"
	EventUnsubscribe    = "unsubscribe"

	// Workflow orchestration events
	EventWorkflowExecute   = "workflow.execute"
	EventAgentSpawned      = "agent.spawned"
	EventAgentThinking     = "agent.thinking"
	EventAgentMemoryUpdate = "agent.memory.update"
	EventAgentStepComplete = "agent.step.complete"
	EventAgentStepFailed   = "agent.step.failed"
	EventWorkflowComplete  = "workflow.complete"
	EventWorkflowFailed    = "workflow.failed"
	EventSharedMemorySync  = "shared.memory.sync"

	// Quick Check events
	EventQuickCheckSubmit    = "quick_check.submit"
	EventQuickCheckAccepted  = "quick_check.accepted"
	EventQuickCheckToken     = "quick_check.token"
	EventQuickCheckCompleted = "quick_check.completed"
	EventQuickCheckError     = "quick_check.error"

	// HuggingFace events
	EventHFSearchModels      = "hf.models.search"
	EventHFModelsResult      = "hf.models.result"
	EventHFModelDetail       = "hf.models.detail"
	EventHFModelDetailResult = "hf.models.detail.result"
	EventHFSearchDatasets    = "hf.datasets.search"
	EventHFDatasetsResult    = "hf.datasets.result"
	EventHFDatasetDetail     = "hf.datasets.detail"
	EventHFDatasetDetailResult = "hf.datasets.detail.result"
	EventHFInference         = "hf.inference"
	EventHFInferenceResult   = "hf.inference.result"
	EventHFBatchInference    = "hf.inference.batch"
	EventHFBatchProgress     = "hf.inference.batch.progress"
	EventHFBatchResult       = "hf.inference.batch.result"
	EventHFError             = "hf.error"
)

// Message is the envelope for all WebSocket communication.
type Message struct {
	Event     string          `json:"event"`
	Channel   string          `json:"channel,omitempty"`
	Data      json.RawMessage `json:"data,omitempty"`
	Timestamp time.Time       `json:"timestamp"`
}

// Handler is a callback for a specific event type.
type Handler func(msg Message)

// Conn manages a WebSocket connection to kortecx with automatic
// reconnection, ping/pong keepalive, and event-based subscriptions.
type Conn struct {
	url        string
	apiKey     string
	conn       *websocket.Conn
	mu         sync.RWMutex
	handlers   map[string][]Handler
	done       chan struct{}
	closed     bool
	reconnect  bool
	pingTicker *time.Ticker

	ReconnectInterval time.Duration
	PingInterval      time.Duration
	MaxReconnectTries int
}

// Option configures the WebSocket connection.
type Option func(*Conn)

// WithReconnect enables automatic reconnection.
func WithReconnect(interval time.Duration, maxTries int) Option {
	return func(c *Conn) {
		c.reconnect = true
		c.ReconnectInterval = interval
		c.MaxReconnectTries = maxTries
	}
}

// WithPing sets the ping keepalive interval.
func WithPing(interval time.Duration) Option {
	return func(c *Conn) { c.PingInterval = interval }
}

// WithWSAPIKey sets the API key for WebSocket auth.
func WithWSAPIKey(key string) Option {
	return func(c *Conn) { c.apiKey = key }
}

// Dial establishes a WebSocket connection to the given URL.
func Dial(wsURL string, opts ...Option) (*Conn, error) {
	c := &Conn{
		url:               wsURL,
		handlers:          make(map[string][]Handler),
		done:              make(chan struct{}),
		ReconnectInterval: 5 * time.Second,
		PingInterval:      30 * time.Second,
		MaxReconnectTries: 10,
	}
	for _, o := range opts {
		o(c)
	}

	if err := c.connect(); err != nil {
		return nil, err
	}

	go c.readLoop()
	go c.pingLoop()

	return c, nil
}

func (c *Conn) connect() error {
	header := make(map[string][]string)
	if c.apiKey != "" {
		header["Authorization"] = []string{"Bearer " + c.apiKey}
	}

	dialer := websocket.Dialer{
		HandshakeTimeout: 10 * time.Second,
	}

	conn, _, err := dialer.Dial(c.url, header)
	if err != nil {
		return fmt.Errorf("websocket dial: %w", err)
	}

	c.mu.Lock()
	c.conn = conn
	c.mu.Unlock()

	return nil
}

func (c *Conn) readLoop() {
	for {
		select {
		case <-c.done:
			return
		default:
		}

		c.mu.RLock()
		conn := c.conn
		c.mu.RUnlock()

		if conn == nil {
			time.Sleep(100 * time.Millisecond)
			continue
		}

		_, raw, err := conn.ReadMessage()
		if err != nil {
			if c.closed {
				return
			}
			slog.Error("websocket read failed", slog.String("component", "ws"), slog.String("error", err.Error()))
			if c.reconnect {
				c.tryReconnect()
			}
			continue
		}

		var msg Message
		if err := json.Unmarshal(raw, &msg); err != nil {
			slog.Warn("websocket message unmarshal failed", slog.String("component", "ws"), slog.String("error", err.Error()))
			continue
		}

		c.dispatch(msg)
	}
}

func (c *Conn) pingLoop() {
	c.pingTicker = time.NewTicker(c.PingInterval)
	defer c.pingTicker.Stop()

	for {
		select {
		case <-c.done:
			return
		case <-c.pingTicker.C:
			_ = c.Send(Message{
				Event:     EventPing,
				Timestamp: time.Now(),
			})
		}
	}
}

func (c *Conn) tryReconnect() {
	for i := 0; i < c.MaxReconnectTries; i++ {
		slog.Info("websocket reconnecting", slog.String("component", "ws"), slog.Int("attempt", i+1), slog.Int("maxAttempts", c.MaxReconnectTries))
		time.Sleep(c.ReconnectInterval)

		if err := c.connect(); err != nil {
			slog.Error("websocket reconnect failed", slog.String("component", "ws"), slog.String("error", err.Error()), slog.Int("attempt", i+1))
			continue
		}

		slog.Info("websocket reconnected", slog.String("component", "ws"), slog.Int("attempt", i+1))
		return
	}
	slog.Error("websocket max reconnect attempts reached", slog.String("component", "ws"), slog.Int("maxAttempts", c.MaxReconnectTries))
}

func (c *Conn) dispatch(msg Message) {
	c.mu.RLock()
	defer c.mu.RUnlock()

	// Dispatch to event-specific handlers
	if handlers, ok := c.handlers[msg.Event]; ok {
		for _, h := range handlers {
			go h(msg)
		}
	}

	// Dispatch to wildcard handlers
	if handlers, ok := c.handlers["*"]; ok {
		for _, h := range handlers {
			go h(msg)
		}
	}
}

// On registers a handler for the given event type. Use "*" for all events.
func (c *Conn) On(event string, h Handler) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.handlers[event] = append(c.handlers[event], h)
}

// Send sends a message over the WebSocket.
func (c *Conn) Send(msg Message) error {
	c.mu.RLock()
	conn := c.conn
	c.mu.RUnlock()

	if conn == nil {
		return fmt.Errorf("websocket not connected")
	}

	if msg.Timestamp.IsZero() {
		msg.Timestamp = time.Now()
	}

	data, err := json.Marshal(msg)
	if err != nil {
		return fmt.Errorf("marshaling message: %w", err)
	}

	return conn.WriteMessage(websocket.TextMessage, data)
}

// Subscribe sends a subscription request for a channel.
func (c *Conn) Subscribe(channel string) error {
	return c.Send(Message{
		Event:   EventSubscribe,
		Channel: channel,
	})
}

// Unsubscribe removes a channel subscription.
func (c *Conn) Unsubscribe(channel string) error {
	return c.Send(Message{
		Event:   EventUnsubscribe,
		Channel: channel,
	})
}

// --- Workflow orchestration WebSocket methods ---

// ExecuteWorkflow sends a workflow execution request over WebSocket.
// The server spawns agents and streams events back via agent.* and workflow.* events.
func (c *Conn) ExecuteWorkflow(req any) error {
	return c.sendEvent(EventWorkflowExecute, req)
}

// OnAgentEvent registers a handler for all agent lifecycle events
// (agent.spawned, agent.thinking, agent.memory.update, agent.step.complete, agent.step.failed).
func (c *Conn) OnAgentEvent(h Handler) {
	c.On(EventAgentSpawned, h)
	c.On(EventAgentThinking, h)
	c.On(EventAgentMemoryUpdate, h)
	c.On(EventAgentStepComplete, h)
	c.On(EventAgentStepFailed, h)
}

// OnWorkflowEvent registers a handler for workflow-level events
// (workflow.complete, workflow.failed).
func (c *Conn) OnWorkflowEvent(h Handler) {
	c.On(EventWorkflowComplete, h)
	c.On(EventWorkflowFailed, h)
}

// --- HuggingFace WebSocket methods ---

// HFSearchModels sends a model search request over WebSocket.
func (c *Conn) HFSearchModels(req any) error {
	return c.sendEvent(EventHFSearchModels, req)
}

// HFGetModel sends a model detail request over WebSocket.
func (c *Conn) HFGetModel(req any) error {
	return c.sendEvent(EventHFModelDetail, req)
}

// HFSearchDatasets sends a dataset search request over WebSocket.
func (c *Conn) HFSearchDatasets(req any) error {
	return c.sendEvent(EventHFSearchDatasets, req)
}

// HFGetDataset sends a dataset detail request over WebSocket.
func (c *Conn) HFGetDataset(req any) error {
	return c.sendEvent(EventHFDatasetDetail, req)
}

// HFInfer sends a single inference request over WebSocket.
func (c *Conn) HFInfer(req any) error {
	return c.sendEvent(EventHFInference, req)
}

// HFBatchInfer sends a batch inference request over WebSocket.
// Results stream back via hf.inference.batch.progress and hf.inference.batch.result events.
func (c *Conn) HFBatchInfer(req any) error {
	return c.sendEvent(EventHFBatchInference, req)
}

func (c *Conn) sendEvent(event string, payload any) error {
	data, err := json.Marshal(payload)
	if err != nil {
		return fmt.Errorf("marshaling %s payload: %w", event, err)
	}
	return c.Send(Message{
		Event: event,
		Data:  data,
	})
}

// Close gracefully shuts down the WebSocket connection.
func (c *Conn) Close() error {
	c.mu.Lock()
	defer c.mu.Unlock()

	if c.closed {
		return nil
	}

	c.closed = true
	close(c.done)

	if c.conn != nil {
		return c.conn.Close()
	}
	return nil
}
