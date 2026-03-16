// Package kortecx provides a Go client library for the Kortecx
// multi-agent AI orchestration platform.
//
// It covers three domains:
//   - data: Dataset and training job management
//   - models: Expert deployment and provider configuration
//   - resources: Task queue, workflows, agents, and monitoring for parallel runs
//
// A WebSocket connection (ws package) enables real-time event streaming
// for task progress, agent status changes, and system alerts.
package kortecx

import (
	"github.com/exi/kortecx-go/client"
	"github.com/exi/kortecx-go/data"
	"github.com/exi/kortecx-go/huggingface"
	"github.com/exi/kortecx-go/models"
	"github.com/exi/kortecx-go/resources"
	"github.com/exi/kortecx-go/ws"
)

// Kortecx is the top-level client that provides access to all services.
type Kortecx struct {
	// Data provides dataset and training management.
	Data *data.Service

	// Models provides expert and provider management.
	Models *models.Service

	// Resources provides task, workflow, agent, and monitoring management.
	Resources *resources.Service

	// HuggingFace provides HuggingFace Hub and Inference API integration.
	HuggingFace *huggingface.Service

	// HTTP is the underlying HTTP client.
	HTTP *client.Client
}

// New creates a fully initialized Kortecx client.
// Use WithHuggingFaceKey to enable direct HuggingFace API access.
func New(baseURL string, opts ...client.Option) *Kortecx {
	c := client.New(baseURL, opts...)
	return &Kortecx{
		Data:        data.New(c),
		Models:      models.New(c),
		Resources:   resources.New(c),
		HuggingFace: huggingface.New(c, ""),
		HTTP:        c,
	}
}

// NewWithHF creates a Kortecx client with a HuggingFace API key for
// direct Hub and Inference API access.
func NewWithHF(baseURL, hfAPIKey string, opts ...client.Option) *Kortecx {
	c := client.New(baseURL, opts...)
	return &Kortecx{
		Data:        data.New(c),
		Models:      models.New(c),
		Resources:   resources.New(c),
		HuggingFace: huggingface.New(c, hfAPIKey),
		HTTP:        c,
	}
}

// Dial opens a WebSocket connection to the kortecx server for real-time events.
// The WebSocket URL is derived from the base URL (http->ws, https->wss).
func (k *Kortecx) Dial(opts ...ws.Option) (*ws.Conn, error) {
	wsURL, err := k.HTTP.WebSocketURL()
	if err != nil {
		return nil, err
	}
	return ws.Dial(wsURL, opts...)
}
