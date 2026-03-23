package quorum

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/exi/kortecx-go/ws"
)

// OrchestratorConfig configures the client-side orchestrator for parallel
// agent execution with backpressure, retry, and rate limiting.
type OrchestratorConfig struct {
	MaxParallel    int           // max concurrent agents (default 4)
	RetryLimit     int           // max retries per agent (default 3)
	RetryDelay     time.Duration // base delay between retries (default 2s)
	RequestTimeout time.Duration // per-agent timeout (default 120s)
	BackpressureQ  int           // backpressure queue size (default 100)
}

// DefaultOrchestratorConfig returns sensible production defaults.
func DefaultOrchestratorConfig() OrchestratorConfig {
	return OrchestratorConfig{
		MaxParallel:    4,
		RetryLimit:     3,
		RetryDelay:     2 * time.Second,
		RequestTimeout: 120 * time.Second,
		BackpressureQ:  100,
	}
}

// AgentTask represents a single agent task to be executed by the orchestrator.
type AgentTask struct {
	StepID      string  // unique identifier for this step
	ExpertID    string  // optional expert ID
	Prompt      string  // the task prompt
	System      string  // system instructions
	Model       string  // model name
	Backend     string  // inference backend (e.g. "ollama", "llamacpp")
	Temperature float64 // sampling temperature
	MaxTokens   int     // max output tokens
	ShareMemory bool    // whether to store output in shared memory
}

// AgentResult is the outcome of a single agent execution attempt.
type AgentResult struct {
	StepID     string // step that produced this result
	AgentID    string // agent identifier
	Output     string // agent output text
	TokensUsed int64  // tokens consumed
	DurationMs int64  // wall-clock duration in milliseconds
	Status     string // "success" | "failed" | "recovered" | "submitted"
	Error      string // non-empty on failure
	Attempt    int    // which retry attempt produced this result
}

// SharedMemoryStore provides thread-safe shared context between agents.
// Agents can read outputs from previous steps and share global key-value pairs.
type SharedMemoryStore struct {
	mu      sync.RWMutex
	entries map[string]string // stepID -> output
	globals map[string]string // key -> value
}

// NewSharedMemory creates an empty shared memory store.
func NewSharedMemory() *SharedMemoryStore {
	return &SharedMemoryStore{
		entries: make(map[string]string),
		globals: make(map[string]string),
	}
}

// Set stores an agent's output in shared memory, keyed by step ID.
func (sm *SharedMemoryStore) Set(stepID, output string) {
	sm.mu.Lock()
	defer sm.mu.Unlock()
	sm.entries[stepID] = output
}

// Get retrieves an agent's output from shared memory by step ID.
func (sm *SharedMemoryStore) Get(stepID string) (string, bool) {
	sm.mu.RLock()
	defer sm.mu.RUnlock()
	v, ok := sm.entries[stepID]
	return v, ok
}

// SetGlobal stores a global key-value pair accessible to all agents.
func (sm *SharedMemoryStore) SetGlobal(key, value string) {
	sm.mu.Lock()
	defer sm.mu.Unlock()
	sm.globals[key] = value
}

// GetGlobal retrieves a global value by key.
func (sm *SharedMemoryStore) GetGlobal(key string) (string, bool) {
	sm.mu.RLock()
	defer sm.mu.RUnlock()
	v, ok := sm.globals[key]
	return v, ok
}

// Snapshot returns a point-in-time copy of all step entries in shared memory.
func (sm *SharedMemoryStore) Snapshot() map[string]string {
	sm.mu.RLock()
	defer sm.mu.RUnlock()
	snap := make(map[string]string, len(sm.entries))
	for k, v := range sm.entries {
		snap[k] = v
	}
	return snap
}

// Orchestrator manages parallel agent execution with semaphore-based
// backpressure, exponential-backoff retries, and shared memory.
type Orchestrator struct {
	svc    *Service
	config OrchestratorConfig

	// Backpressure: buffered channel acts as a counting semaphore.
	sem chan struct{}

	// Metrics tracked with atomic counters for lock-free reads.
	totalSubmitted atomic.Int64
	totalCompleted atomic.Int64
	totalFailed    atomic.Int64
	totalRetries   atomic.Int64

	// Shared memory for inter-agent communication.
	memory *SharedMemoryStore

	// Optional progress callback for orchestration lifecycle events.
	onProgress func(event string, data map[string]any)

	// Cancel support: calling Cancel() closes this channel to signal all goroutines.
	cancelCh   chan struct{}
	cancelOnce sync.Once
	cancelled  atomic.Bool
}

// NewOrchestrator creates a new orchestrator bound to the given quorum service.
// Invalid config values are replaced with safe defaults.
func NewOrchestrator(svc *Service, cfg OrchestratorConfig) *Orchestrator {
	if cfg.MaxParallel <= 0 {
		cfg.MaxParallel = 4
	}
	if cfg.RetryLimit <= 0 {
		cfg.RetryLimit = 3
	}
	if cfg.RetryDelay <= 0 {
		cfg.RetryDelay = 2 * time.Second
	}
	if cfg.RequestTimeout <= 0 {
		cfg.RequestTimeout = 120 * time.Second
	}
	if cfg.BackpressureQ <= 0 {
		cfg.BackpressureQ = 100
	}
	return &Orchestrator{
		svc:      svc,
		config:   cfg,
		sem:      make(chan struct{}, cfg.MaxParallel),
		memory:   NewSharedMemory(),
		cancelCh: make(chan struct{}),
	}
}

// OnProgress sets a callback for orchestration progress events.
// Events include: agent.queued, agent.attempt, agent.retry, agent.exhausted,
// agent.started, chain.broken.
func (o *Orchestrator) OnProgress(fn func(event string, data map[string]any)) {
	o.onProgress = fn
}

// Memory returns the shared memory store used by this orchestrator.
func (o *Orchestrator) Memory() *SharedMemoryStore {
	return o.memory
}

// Stats returns current orchestrator metrics as a snapshot.
func (o *Orchestrator) Stats() map[string]int64 {
	return map[string]int64{
		"submitted": o.totalSubmitted.Load(),
		"completed": o.totalCompleted.Load(),
		"failed":    o.totalFailed.Load(),
		"retries":   o.totalRetries.Load(),
		"active":    int64(len(o.sem)),
		"queued":    int64(len(o.sem)),
	}
}

// Cancel signals all running goroutines to stop. Safe to call multiple times.
func (o *Orchestrator) Cancel() {
	o.cancelOnce.Do(func() {
		o.cancelled.Store(true)
		close(o.cancelCh)
		o.emit("workflow.cancelled", map[string]any{
			"message": "Orchestrator cancelled by user",
		})
	})
}

// IsCancelled reports whether Cancel has been called.
func (o *Orchestrator) IsCancelled() bool {
	return o.cancelled.Load()
}

// Reset prepares the orchestrator for reuse after cancellation.
func (o *Orchestrator) Reset() {
	o.cancelCh = make(chan struct{})
	o.cancelOnce = sync.Once{}
	o.cancelled.Store(false)
	o.totalSubmitted.Store(0)
	o.totalCompleted.Store(0)
	o.totalFailed.Store(0)
	o.totalRetries.Store(0)
}

// ExecuteParallel runs multiple agent tasks concurrently with backpressure.
// The semaphore limits concurrency to MaxParallel. Results are returned in
// the same order as the input tasks. Cancelling the context aborts pending work.
func (o *Orchestrator) ExecuteParallel(ctx context.Context, tasks []AgentTask) []AgentResult {
	results := make([]AgentResult, len(tasks))
	resultChs := make([]chan AgentResult, len(tasks))

	for i := range tasks {
		resultChs[i] = make(chan AgentResult, 1)
	}

	// Inject shared memory context into parallel tasks that opt in.
	snapshot := o.memory.Snapshot()
	for i := range tasks {
		if tasks[i].ShareMemory && len(snapshot) > 0 {
			var sb strings.Builder
			for sid, out := range snapshot {
				sb.WriteString(fmt.Sprintf("## Step %s Output\n%s\n\n", sid, truncate(out, 1500)))
			}
			tasks[i].Prompt = sb.String() + "## Your Task\n" + tasks[i].Prompt
		}
	}

	// Fan out: one goroutine per task, gated by the semaphore.
	for i, task := range tasks {
		o.totalSubmitted.Add(1)
		o.emit("agent.queued", map[string]any{
			"stepId": task.StepID, "position": i, "total": len(tasks),
		})

		go func(t AgentTask, ch chan<- AgentResult) {
			// Backpressure: block until a semaphore slot is available.
			select {
			case o.sem <- struct{}{}:
			case <-ctx.Done():
				ch <- AgentResult{StepID: t.StepID, Status: "failed", Error: "cancelled"}
				return
			case <-o.cancelCh:
				ch <- AgentResult{StepID: t.StepID, Status: "failed", Error: "cancelled"}
				return
			}
			defer func() { <-o.sem }()

			// Check cancellation before executing
			if o.IsCancelled() {
				ch <- AgentResult{StepID: t.StepID, Status: "failed", Error: "cancelled"}
				return
			}

			result := o.executeWithRetry(ctx, t)

			if t.ShareMemory && (result.Status == "success" || result.Status == "submitted") {
				o.memory.Set(t.StepID, result.Output)
			}

			ch <- result
		}(task, resultChs[i])
	}

	// Collect results in order.
	for i, ch := range resultChs {
		select {
		case r := <-ch:
			results[i] = r
			if r.Status == "success" || r.Status == "recovered" || r.Status == "submitted" {
				o.totalCompleted.Add(1)
			} else {
				o.totalFailed.Add(1)
			}
		case <-ctx.Done():
			results[i] = AgentResult{StepID: tasks[i].StepID, Status: "failed", Error: "timeout"}
			o.totalFailed.Add(1)
		}
	}

	return results
}

// ExecuteSequential runs tasks one after another, passing each previous output
// as context to the next task when ShareMemory is enabled. A failure in any
// step breaks the chain and stops further execution.
func (o *Orchestrator) ExecuteSequential(ctx context.Context, tasks []AgentTask) []AgentResult {
	results := make([]AgentResult, 0, len(tasks))
	var previousOutput string

	for i, task := range tasks {
		// Check cancellation before each step
		if o.IsCancelled() {
			o.emit("workflow.cancelled", map[string]any{
				"stepId": task.StepID, "step": i + 1,
			})
			break
		}

		o.totalSubmitted.Add(1)

		// Inject previous output into prompt for chained reasoning.
		if previousOutput != "" && task.ShareMemory {
			task.Prompt = fmt.Sprintf("## Previous Step Output\n%s\n\n## Your Task\n%s",
				truncate(previousOutput, 3000), task.Prompt)
		}

		o.emit("agent.started", map[string]any{
			"stepId": task.StepID, "step": i + 1, "total": len(tasks),
		})

		result := o.executeWithRetry(ctx, task)
		results = append(results, result)

		if result.Status == "success" || result.Status == "recovered" || result.Status == "submitted" {
			o.totalCompleted.Add(1)
			previousOutput = result.Output
			if task.ShareMemory {
				o.memory.Set(task.StepID, result.Output)
			}
		} else {
			o.totalFailed.Add(1)
			o.emit("chain.broken", map[string]any{
				"stepId": task.StepID, "error": result.Error, "step": i + 1,
			})
			break
		}
	}

	return results
}

// executeWithRetry attempts to execute a single agent task with exponential
// backoff. On each failure the delay doubles: RetryDelay * attempt.
func (o *Orchestrator) executeWithRetry(ctx context.Context, task AgentTask) AgentResult {
	var lastErr string

	for attempt := 1; attempt <= o.config.RetryLimit; attempt++ {
		o.emit("agent.attempt", map[string]any{
			"stepId": task.StepID, "attempt": attempt, "maxAttempts": o.config.RetryLimit,
		})

		submitData := map[string]any{
			"project":     "workflow",
			"task":        task.Prompt,
			"model":       task.Model,
			"backend":     task.Backend,
			"workers":     1,
			"prompt":      task.System,
			"temperature": task.Temperature,
			"max_tokens":  task.MaxTokens,
			"retries":     1,
		}

		data, err := json.Marshal(submitData)
		if err != nil {
			lastErr = fmt.Sprintf("marshal failed: %v", err)
			continue
		}

		sendErr := o.svc.ws.Send(ws.Message{
			Event: EventSubmitRun,
			Data:  data,
		})

		if sendErr != nil {
			lastErr = fmt.Sprintf("send failed: %v", sendErr)
			o.totalRetries.Add(1)
			o.emit("agent.retry", map[string]any{
				"stepId": task.StepID, "attempt": attempt, "error": lastErr,
			})

			if attempt < o.config.RetryLimit {
				// Exponential backoff: base delay * attempt number.
				delay := o.config.RetryDelay * time.Duration(attempt)
				select {
				case <-time.After(delay):
				case <-ctx.Done():
					return AgentResult{
						StepID:  task.StepID,
						Status:  "failed",
						Error:   "cancelled",
						Attempt: attempt,
					}
				}
			}
			continue
		}

		// Send succeeded. Actual results arrive asynchronously via WebSocket
		// event handlers (OnRunEvent / OnAgentEvent).
		return AgentResult{
			StepID:  task.StepID,
			AgentID: fmt.Sprintf("agent-%s-%d", task.StepID, attempt),
			Status:  "submitted",
			Attempt: attempt,
		}
	}

	// All retry attempts exhausted.
	o.emit("agent.exhausted", map[string]any{
		"stepId": task.StepID, "error": lastErr, "attempts": o.config.RetryLimit,
	})

	return AgentResult{
		StepID:  task.StepID,
		Status:  "failed",
		Error:   fmt.Sprintf("all %d attempts failed: %s", o.config.RetryLimit, lastErr),
		Attempt: o.config.RetryLimit,
	}
}

// emit dispatches a progress event to the registered callback, if any.
func (o *Orchestrator) emit(event string, data map[string]any) {
	if o.onProgress != nil {
		o.onProgress(event, data)
	}
}

// truncate shortens a string to at most n bytes, appending "..." if truncated.
func truncate(s string, n int) string {
	if len(s) <= n {
		return s
	}
	return s[:n] + "..."
}
