package kortecx_test

import (
	"fmt"
	"log"
	"time"

	kortecx "github.com/exi/kortecx-go"
	"github.com/exi/kortecx-go/client"
	"github.com/exi/kortecx-go/models"
	"github.com/exi/kortecx-go/types"
	"github.com/exi/kortecx-go/ws"
)

func Example() {
	// Initialize the client
	k := kortecx.New("http://localhost:3000",
		client.WithAPIKey("your-api-key"),
	)

	// --- Data Management ---

	// Create a dataset for fine-tuning
	ds, err := k.Data.CreateDataset(types.CreateDatasetRequest{
		Name:        "customer-support-v2",
		Description: "Customer support conversation pairs",
		Format:      types.FormatJSONL,
		Tags:        []string{"support", "production"},
	})
	if err != nil {
		log.Fatal(err)
	}
	fmt.Printf("Dataset created: %s\n", ds.ID)

	// Start a training job
	job, err := k.Data.StartTraining(types.StartTrainingRequest{
		Name:         "support-expert-finetune",
		ExpertID:     "expert-support-01",
		BaseModelID:  "claude-sonnet-4-6",
		DatasetID:    ds.ID,
		Epochs:       3,
		LearningRate: 0.0001,
		BatchSize:    32,
	})
	if err != nil {
		log.Fatal(err)
	}
	fmt.Printf("Training started: %s (status: %s)\n", job.ID, job.Status)

	// --- Model Handling ---

	// List all coder experts
	experts, err := k.Models.ListExperts(models.WithRole(types.RoleCoder))
	if err != nil {
		log.Fatal(err)
	}
	for _, e := range experts {
		fmt.Printf("Expert: %s (%s) - success rate: %.1f%%\n", e.Name, e.Status, e.SuccessRate)
	}

	// Deploy a new expert
	expert, err := k.Models.DeployExpert(types.DeployExpertRequest{
		Name:         "code-reviewer-v2",
		Role:         types.RoleReviewer,
		ModelID:      "claude-sonnet-4-6",
		ProviderID:   "anthropic",
		SystemPrompt: "You are a senior code reviewer. Focus on correctness, security, and performance.",
		Temperature:  0.3,
		MaxTokens:    4096,
		ReplicaCount: 2,
	})
	if err != nil {
		log.Fatal(err)
	}
	fmt.Printf("Expert deployed: %s\n", expert.ID)

	// --- Resource Management for Parallel Runs ---

	// Create a workflow
	wf, err := k.Resources.CreateWorkflow(types.CreateWorkflowRequest{
		Name:          "PR Review Pipeline",
		Description:   "Automated code review with multiple expert passes",
		GoalStatement: "Review PRs for correctness, security, and style",
		Tags:          []string{"ci", "review"},
	})
	if err != nil {
		log.Fatal(err)
	}

	// Kick off parallel tasks
	for i, input := range []string{"review auth module", "review API handlers", "review DB queries"} {
		task, err := k.Resources.CreateTask(types.CreateTaskRequest{
			Name:       fmt.Sprintf("review-task-%d", i+1),
			WorkflowID: wf.ID,
			Priority:   types.PriorityHigh,
			Input:      input,
		})
		if err != nil {
			log.Fatal(err)
		}
		fmt.Printf("Task queued: %s (priority: %s)\n", task.ID, task.Priority)
	}

	// Start the workflow run
	run, err := k.Resources.RunWorkflow(wf.ID)
	if err != nil {
		log.Fatal(err)
	}
	fmt.Printf("Workflow run started: %s\n", run.ID)

	// --- Real-time WebSocket Monitoring ---

	conn, err := k.Dial(
		ws.WithReconnect(5*time.Second, 10),
		ws.WithPing(30*time.Second),
	)
	if err != nil {
		log.Fatal(err)
	}
	defer conn.Close()

	// Subscribe to task and run events
	conn.Subscribe("tasks")
	conn.Subscribe("runs")

	// Handle task completions
	conn.On(ws.EventTaskCompleted, func(msg ws.Message) {
		fmt.Printf("Task completed: %s\n", string(msg.Data))
	})

	// Handle failures
	conn.On(ws.EventTaskFailed, func(msg ws.Message) {
		fmt.Printf("Task failed: %s\n", string(msg.Data))
	})

	// Handle run completion
	conn.On(ws.EventRunCompleted, func(msg ws.Message) {
		fmt.Printf("Workflow run finished: %s\n", string(msg.Data))
	})

	// Monitor agents
	agents, err := k.Resources.ListAgents()
	if err != nil {
		log.Fatal(err)
	}
	fmt.Printf("Active agents: %d\n", len(agents))

	// Check system health
	monitoring, err := k.Resources.GetMonitoring()
	if err != nil {
		log.Fatal(err)
	}
	fmt.Printf("Success rate: %.1f%%, Active agents: %d\n",
		monitoring.Metrics.SuccessRate, monitoring.Metrics.ActiveAgents)

	// --- HuggingFace Integration ---

	// Search for text-generation models via REST
	hfModels, err := k.HuggingFace.SearchModels(types.HFSearchModelsRequest{
		Search:   "llama",
		Pipeline: types.HFPipelineTextGeneration,
		Sort:     "downloads",
		Limit:    10,
	})
	if err != nil {
		log.Fatal(err)
	}
	for _, m := range hfModels {
		fmt.Printf("HF Model: %s (downloads: %d)\n", m.ModelID, m.Downloads)
	}

	// Search datasets via REST
	hfDatasets, err := k.HuggingFace.SearchDatasets(types.HFSearchDatasetsRequest{
		Search: "code-instruct",
		Sort:   "likes",
		Limit:  5,
	})
	if err != nil {
		log.Fatal(err)
	}
	for _, d := range hfDatasets {
		fmt.Printf("HF Dataset: %s (likes: %d)\n", d.ID, d.Likes)
	}

	// Run single inference via REST
	inferResp, err := k.HuggingFace.RunInference(types.HFInferenceRequest{
		RequestID: "req-001",
		Model:     "meta-llama/Llama-2-7b-chat-hf",
		Inputs:    "Explain quantum computing in simple terms.",
		Parameters: map[string]any{
			"max_new_tokens": 256,
			"temperature":    0.7,
		},
	})
	if err != nil {
		log.Fatal(err)
	}
	fmt.Printf("Inference %s: status=%s duration=%dms\n",
		inferResp.RequestID, inferResp.Status, inferResp.DurationMs)

	// --- HuggingFace via WebSocket (parallel at scale) ---

	// Search models over WebSocket
	conn.HFSearchModels(types.HFSearchModelsRequest{
		RequestID: "ws-search-001",
		Search:    "mistral",
		Pipeline:  types.HFPipelineTextGeneration,
		Limit:     20,
	})

	// Handle model search results
	conn.On(ws.EventHFModelsResult, func(msg ws.Message) {
		fmt.Printf("HF model search results: %s\n", string(msg.Data))
	})

	// Batch inference over WebSocket — server processes all in parallel
	conn.HFBatchInfer(types.HFBatchInferenceRequest{
		RequestID: "batch-001",
		Requests: []types.HFInferenceRequest{
			{RequestID: "b1", Model: "meta-llama/Llama-2-7b-chat-hf", Inputs: "What is Go?"},
			{RequestID: "b2", Model: "meta-llama/Llama-2-7b-chat-hf", Inputs: "What is Rust?"},
			{RequestID: "b3", Model: "mistralai/Mistral-7B-v0.1", Inputs: "What is Python?"},
		},
	})

	// Stream individual results as they complete
	conn.On(ws.EventHFBatchProgress, func(msg ws.Message) {
		fmt.Printf("Batch progress: %s\n", string(msg.Data))
	})

	// Final batch result
	conn.On(ws.EventHFBatchResult, func(msg ws.Message) {
		fmt.Printf("Batch complete: %s\n", string(msg.Data))
	})

	// Handle HuggingFace errors
	conn.On(ws.EventHFError, func(msg ws.Message) {
		fmt.Printf("HF error: %s\n", string(msg.Data))
	})
}
