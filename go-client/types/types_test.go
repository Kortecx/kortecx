package types

import (
	"encoding/json"
	"testing"
)

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

func TestAssetJSON(t *testing.T) {
	asset := Asset{
		ID:          "asset-123",
		Name:        "response_20260322.md",
		Folder:      "/experts/2026-03-22/research-analyst",
		MimeType:    "text/markdown",
		FileType:    "document",
		FilePath:    "/data/experts/local/2026-03-22/research-analyst/response_20260322.md",
		FileName:    "response_20260322.md",
		SizeBytes:   4096,
		Tags:        []string{"researcher", "expert-run"},
		ExpertID:    "exp-research",
		ExpertRunID: "run-abc123",
		SourceType:  "expert",
	}

	data, err := json.Marshal(asset)
	if err != nil {
		t.Fatalf("marshal Asset: %v", err)
	}

	var decoded Asset
	if err := json.Unmarshal(data, &decoded); err != nil {
		t.Fatalf("unmarshal Asset: %v", err)
	}

	if decoded.ID != asset.ID {
		t.Errorf("ID = %q, want %q", decoded.ID, asset.ID)
	}
	if decoded.ExpertID != "exp-research" {
		t.Errorf("ExpertID = %q, want \"exp-research\"", decoded.ExpertID)
	}
	if decoded.SourceType != "expert" {
		t.Errorf("SourceType = %q, want \"expert\"", decoded.SourceType)
	}
	if decoded.SizeBytes != 4096 {
		t.Errorf("SizeBytes = %d, want 4096", decoded.SizeBytes)
	}
	if len(decoded.Tags) != 2 {
		t.Errorf("Tags length = %d, want 2", len(decoded.Tags))
	}
}

func TestAssetListOptions(t *testing.T) {
	opts := AssetListOptions{
		Folder:     "/experts",
		ExpertID:   "exp-1",
		SourceType: "expert",
		Search:     "response",
	}

	data, err := json.Marshal(opts)
	if err != nil {
		t.Fatalf("marshal AssetListOptions: %v", err)
	}

	var decoded AssetListOptions
	if err := json.Unmarshal(data, &decoded); err != nil {
		t.Fatalf("unmarshal AssetListOptions: %v", err)
	}

	if decoded.SourceType != "expert" {
		t.Errorf("SourceType = %q, want \"expert\"", decoded.SourceType)
	}
}

func TestExpertRunJSON(t *testing.T) {
	run := ExpertRun{
		ID:           "er-abc123",
		ExpertID:     "exp-research",
		ExpertName:   "Research Analyst",
		Status:       "completed",
		Model:        "llama3.2:3b",
		Engine:       "ollama",
		TokensUsed:   500,
		DurationMs:   2000,
		ArtifactCount: 4,
		ResponseText: "Generated analysis...",
	}

	data, err := json.Marshal(run)
	if err != nil {
		t.Fatalf("marshal ExpertRun: %v", err)
	}

	var decoded ExpertRun
	if err := json.Unmarshal(data, &decoded); err != nil {
		t.Fatalf("unmarshal ExpertRun: %v", err)
	}

	if decoded.Status != "completed" {
		t.Errorf("Status = %q, want \"completed\"", decoded.Status)
	}
	if decoded.ArtifactCount != 4 {
		t.Errorf("ArtifactCount = %d, want 4", decoded.ArtifactCount)
	}
	if decoded.ExpertName != "Research Analyst" {
		t.Errorf("ExpertName = %q, want \"Research Analyst\"", decoded.ExpertName)
	}
}

func TestRunExpertRequestJSON(t *testing.T) {
	req := RunExpertRequest{
		ExpertID:   "exp-1",
		ExpertName: "Test Expert",
		Model:      "llama3.2:3b",
		Engine:     "ollama",
		Role:       "researcher",
		Tags:       []string{"demo"},
	}

	data, err := json.Marshal(req)
	if err != nil {
		t.Fatalf("marshal RunExpertRequest: %v", err)
	}

	var decoded RunExpertRequest
	if err := json.Unmarshal(data, &decoded); err != nil {
		t.Fatalf("unmarshal RunExpertRequest: %v", err)
	}

	if decoded.ExpertID != "exp-1" {
		t.Errorf("ExpertID = %q, want \"exp-1\"", decoded.ExpertID)
	}
}

func TestRegisterAssetsRequest(t *testing.T) {
	req := RegisterAssetsRequest{
		Assets: []Asset{
			{ID: "a1", Name: "file1.md", SourceType: "expert"},
			{ID: "a2", Name: "file2.json", SourceType: "expert"},
		},
	}

	data, err := json.Marshal(req)
	if err != nil {
		t.Fatalf("marshal RegisterAssetsRequest: %v", err)
	}

	var decoded RegisterAssetsRequest
	if err := json.Unmarshal(data, &decoded); err != nil {
		t.Fatalf("unmarshal RegisterAssetsRequest: %v", err)
	}

	if len(decoded.Assets) != 2 {
		t.Errorf("Assets length = %d, want 2", len(decoded.Assets))
	}
}
