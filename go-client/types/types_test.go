package types

import "testing"

func TestProviderConstants(t *testing.T) {
	providers := []ProviderID{
		ProviderAnthropic, ProviderOpenAI, ProviderGoogle,
		ProviderHuggingFace, ProviderDeepSeek, ProviderXAI,
	}
	seen := map[ProviderID]bool{}
	for _, p := range providers {
		if seen[p] {
			t.Errorf("duplicate provider: %s", p)
		}
		seen[p] = true
		if p == "" {
			t.Error("empty provider ID")
		}
	}
}

func TestTaskStatusConstants(t *testing.T) {
	statuses := []TaskStatus{
		TaskQueued, TaskRunning, TaskCompleted, TaskFailed, TaskCancelled,
	}
	if len(statuses) != 5 {
		t.Errorf("expected 5 task statuses, got %d", len(statuses))
	}
}

func TestModelSourceConstants(t *testing.T) {
	if ModelSourceLocal != "local" {
		t.Errorf("ModelSourceLocal = %q, want \"local\"", ModelSourceLocal)
	}
	if ModelSourceProvider != "provider" {
		t.Errorf("ModelSourceProvider = %q, want \"provider\"", ModelSourceProvider)
	}
}
