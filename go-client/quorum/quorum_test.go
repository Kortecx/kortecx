package quorum

import (
	"context"
	"encoding/json"
	"testing"
	"time"
)

// ── Type Tests ────────────────────────────────────────

func TestSubmitRequestJSON(t *testing.T) {
	req := SubmitRequest{
		Project:     "test-project",
		Task:        "analyze data",
		Model:       "llama3.2:3b",
		Backend:     "ollama",
		Workers:     3,
		Temperature: 0.7,
		MaxTokens:   2048,
		Retries:     3,
	}
	data, err := json.Marshal(req)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}
	var decoded SubmitRequest
	if err := json.Unmarshal(data, &decoded); err != nil {
		t.Fatalf("unmarshal failed: %v", err)
	}
	if decoded.Project != "test-project" {
		t.Errorf("expected project 'test-project', got '%s'", decoded.Project)
	}
	if decoded.Workers != 3 {
		t.Errorf("expected 3 workers, got %d", decoded.Workers)
	}
}

func TestRunResultJSON(t *testing.T) {
	result := RunResult{
		RunID:            "test-run-id",
		TotalTokens:      5000,
		TotalDurationMs:  15000,
		DecomposeMs:      3000,
		ExecuteMs:        10000,
		SynthesizeMs:     2000,
		FinalOutput:      "test output",
		WorkersSucceeded: 3,
		WorkersFailed:    0,
		WorkersRecovered: 0,
	}
	data, err := json.Marshal(result)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}
	if len(data) == 0 {
		t.Fatal("empty JSON output")
	}
	var decoded RunResult
	if err := json.Unmarshal(data, &decoded); err != nil {
		t.Fatalf("unmarshal failed: %v", err)
	}
	if decoded.TotalTokens != 5000 {
		t.Errorf("expected 5000 tokens, got %d", decoded.TotalTokens)
	}
}

func TestPhaseUpdateJSON(t *testing.T) {
	update := PhaseUpdate{
		RunID:           "run-1",
		Phase:           "execute",
		Status:          "complete",
		WallClockMs:     15000,
		SumIndividualMs: 25000,
		Speedup:         1.67,
		Parallel:        true,
	}
	data, err := json.Marshal(update)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}
	var decoded PhaseUpdate
	json.Unmarshal(data, &decoded)
	if !decoded.Parallel {
		t.Error("expected parallel to be true")
	}
	if decoded.Speedup != 1.67 {
		t.Errorf("expected speedup 1.67, got %f", decoded.Speedup)
	}
}

func TestAgentCreatedJSON(t *testing.T) {
	agent := AgentCreated{
		RunID:   "run-1",
		AgentID: "worker_1",
		Role:    "worker",
		Subtask: "research topic",
		Model:   "llama3.2:3b",
	}
	data, _ := json.Marshal(agent)
	var decoded AgentCreated
	json.Unmarshal(data, &decoded)
	if decoded.Role != "worker" {
		t.Errorf("expected role 'worker', got '%s'", decoded.Role)
	}
}

func TestExpertRunRequestJSON(t *testing.T) {
	req := ExpertRunRequest{
		Name:       "test-workflow",
		WorkflowID: "wf-123",
		Steps: []ExpertStepConfig{
			{
				StepID:          "step-1",
				ExpertID:        "exp-1",
				TaskDescription: "do something",
				ModelSource:     "local",
				LocalModel: &LocalModelConfig{
					Engine: "ollama",
					Model:  "llama3.2:3b",
				},
				Temperature:    0.5,
				MaxTokens:      4096,
				ConnectionType: "parallel",
			},
		},
	}
	data, err := json.Marshal(req)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}
	var decoded ExpertRunRequest
	json.Unmarshal(data, &decoded)
	if len(decoded.Steps) != 1 {
		t.Fatalf("expected 1 step, got %d", len(decoded.Steps))
	}
	if decoded.Steps[0].LocalModel.Engine != "ollama" {
		t.Error("expected ollama engine")
	}
}

func TestMetricsSnapshotJSON(t *testing.T) {
	snap := MetricsSnapshot{
		ActiveRuns:    2,
		QueuedRuns:    3,
		MaxConcurrent: 4,
		CPUUsage:      45.5,
		TokensPerSec:  12.3,
	}
	data, _ := json.Marshal(snap)
	var decoded MetricsSnapshot
	json.Unmarshal(data, &decoded)
	if decoded.ActiveRuns != 2 {
		t.Errorf("expected 2 active runs, got %d", decoded.ActiveRuns)
	}
}

func TestLiveMetricsJSON(t *testing.T) {
	metrics := LiveMetrics{
		ActiveAgents: 5,
		ActiveRuns:   2,
		TokensUsed:   10000,
		SuccessRate:  0.95,
		ActiveModels: map[string]int{"llama3.2:3b": 2},
	}
	data, _ := json.Marshal(metrics)
	var decoded LiveMetrics
	json.Unmarshal(data, &decoded)
	if decoded.SuccessRate != 0.95 {
		t.Errorf("expected 0.95 success rate, got %f", decoded.SuccessRate)
	}
	if decoded.ActiveModels["llama3.2:3b"] != 2 {
		t.Error("expected 2 active model instances")
	}
}

// ── Orchestrator Tests ────────────────────────────────

func TestSharedMemory(t *testing.T) {
	sm := NewSharedMemory()

	// Set and get
	sm.Set("step-1", "output from step 1")
	val, ok := sm.Get("step-1")
	if !ok || val != "output from step 1" {
		t.Error("failed to get step-1 output")
	}

	// Get nonexistent
	_, ok = sm.Get("nonexistent")
	if ok {
		t.Error("expected false for nonexistent key")
	}

	// Globals
	sm.SetGlobal("key1", "value1")
	gval, ok := sm.GetGlobal("key1")
	if !ok || gval != "value1" {
		t.Error("failed to get global key1")
	}

	// Snapshot
	snap := sm.Snapshot()
	if snap["step-1"] != "output from step 1" {
		t.Error("snapshot missing step-1")
	}

	// Snapshot is a copy
	snap["step-1"] = "modified"
	original, _ := sm.Get("step-1")
	if original != "output from step 1" {
		t.Error("snapshot should be a copy, not a reference")
	}
}

func TestOrchestratorConfig(t *testing.T) {
	cfg := DefaultOrchestratorConfig()
	if cfg.MaxParallel != 4 {
		t.Errorf("expected 4 max parallel, got %d", cfg.MaxParallel)
	}
	if cfg.RetryLimit != 3 {
		t.Errorf("expected 3 retry limit, got %d", cfg.RetryLimit)
	}
	if cfg.RetryDelay != 2*time.Second {
		t.Errorf("expected 2s retry delay, got %v", cfg.RetryDelay)
	}
}

func TestOrchestratorStats(t *testing.T) {
	// Can't create a full orchestrator without ws.Conn, but we can test the config
	cfg := DefaultOrchestratorConfig()
	cfg.MaxParallel = 8
	cfg.BackpressureQ = 50
	if cfg.MaxParallel != 8 {
		t.Error("config override failed")
	}
}

func TestTruncate(t *testing.T) {
	tests := []struct {
		input    string
		n        int
		expected string
	}{
		{"hello", 10, "hello"},
		{"hello world", 5, "hello..."},
		{"", 5, ""},
		{"ab", 2, "ab"},
		{"abc", 2, "ab..."},
	}
	for _, tc := range tests {
		result := truncate(tc.input, tc.n)
		if result != tc.expected {
			t.Errorf("truncate(%q, %d) = %q, want %q", tc.input, tc.n, result, tc.expected)
		}
	}
}

func TestAgentTaskFields(t *testing.T) {
	task := AgentTask{
		StepID:      "step-1",
		ExpertID:    "expert-1",
		Prompt:      "do analysis",
		System:      "you are an analyst",
		Model:       "llama3.2:3b",
		Backend:     "ollama",
		Temperature: 0.5,
		MaxTokens:   2048,
		ShareMemory: true,
	}
	if !task.ShareMemory {
		t.Error("expected share memory to be true")
	}
}

func TestAgentResult(t *testing.T) {
	result := AgentResult{
		StepID:     "step-1",
		AgentID:    "agent-1",
		Output:     "analysis complete",
		TokensUsed: 500,
		DurationMs: 3000,
		Status:     "success",
		Attempt:    1,
	}
	if result.Status != "success" {
		t.Error("expected success status")
	}
	data, err := json.Marshal(result)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}
	if len(data) == 0 {
		t.Fatal("empty JSON")
	}
}

// ── Event Constants ──────────────────────────────────

func TestEventConstants(t *testing.T) {
	events := []string{
		EventSubmitRun, EventCancelRun, EventRunStatus,
		EventRunQueued, EventRunStarted, EventRunComplete, EventRunFailed,
		EventPhaseUpdate, EventAgentCreated, EventAgentThinking,
		EventAgentOutput, EventAgentFailed, EventAgentRecovered,
		EventMetricsSnap, EventQuorumError,
		EventExpertStatsUpdate, EventWorkflowExecute,
	}
	for _, e := range events {
		if e == "" {
			t.Error("event constant should not be empty")
		}
		if len(e) < 5 {
			t.Errorf("event '%s' seems too short", e)
		}
	}
}

// ── Graph Type Tests ─────────────────────────────────

func TestSimilarityEdge_JSON(t *testing.T) {
	edge := SimilarityEdge{Source: "prism-a", Target: "prism-b", Weight: 0.85}
	data, err := json.Marshal(edge)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	var got SimilarityEdge
	if err := json.Unmarshal(data, &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got.Source != "prism-a" || got.Target != "prism-b" || got.Weight != 0.85 {
		t.Fatalf("unexpected: %+v", got)
	}
}

func TestGraphEdgesResponse_JSON(t *testing.T) {
	resp := GraphEdgesResponse{
		Edges:   []SimilarityEdge{{Source: "a", Target: "b", Weight: 0.9}},
		Total:   1,
		Version: "3-12345",
	}
	data, err := json.Marshal(resp)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	var got GraphEdgesResponse
	if err := json.Unmarshal(data, &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got.Total != 1 || len(got.Edges) != 1 || got.Version != "3-12345" {
		t.Fatalf("unexpected: %+v", got)
	}
}

func TestEmbedAssetsRequest_JSON(t *testing.T) {
	req := EmbedAssetsRequest{FileTexts: []string{"hello world", "test content"}}
	data, err := json.Marshal(req)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	var got EmbedAssetsRequest
	if err := json.Unmarshal(data, &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if len(got.FileTexts) != 2 || got.FileTexts[0] != "hello world" {
		t.Fatalf("unexpected: %+v", got)
	}
}

func TestEmbedBulkRequest_JSON(t *testing.T) {
	req := EmbedBulkRequest{
		Experts: []map[string]interface{}{{"id": "mp-1", "name": "Test"}},
		Source:  "marketplace",
	}
	data, err := json.Marshal(req)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	var got EmbedBulkRequest
	if err := json.Unmarshal(data, &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got.Source != "marketplace" || len(got.Experts) != 1 {
		t.Fatalf("unexpected: %+v", got)
	}
}

// ── Context cancellation ─────────────────────────────

func TestContextCancellation(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	select {
	case <-ctx.Done():
		// Expected
	default:
		t.Error("context should be cancelled")
	}
}
