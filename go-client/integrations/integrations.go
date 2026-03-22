// Package integrations provides integration and plugin management for kortecx,
// including external service connections, connection testing, and plugin installation.
package integrations

import (
	"net/http"

	"github.com/exi/kortecx-go/client"
	"github.com/exi/kortecx-go/types"
)

// Service handles integration and plugin management operations.
type Service struct {
	c *client.Client
}

// New creates a new integrations service.
func New(c *client.Client) *Service {
	return &Service{c: c}
}

// ListIntegrations returns all available integrations.
func (s *Service) ListIntegrations() ([]types.Integration, error) {
	var out []types.Integration
	err := s.c.Do(http.MethodGet, "/api/integrations", nil, &out)
	return out, err
}

// GetIntegration returns a specific integration by ID.
func (s *Service) GetIntegration(id string) (*types.Integration, error) {
	var out types.Integration
	err := s.c.Do(http.MethodGet, "/api/integrations/"+id, nil, &out)
	return &out, err
}

// ListConnections returns all configured integration connections.
func (s *Service) ListConnections() ([]types.IntegrationConnection, error) {
	var out []types.IntegrationConnection
	err := s.c.Do(http.MethodGet, "/api/integrations/connections", nil, &out)
	return out, err
}

// CreateConnection creates a new integration connection.
func (s *Service) CreateConnection(req types.CreateConnectionRequest) (*types.IntegrationConnection, error) {
	var out types.IntegrationConnection
	err := s.c.Do(http.MethodPost, "/api/integrations/connections", req, &out)
	return &out, err
}

// TestConnection tests an existing integration connection by ID.
func (s *Service) TestConnection(id string) (*types.ConnectionTestResult, error) {
	var out types.ConnectionTestResult
	err := s.c.Do(http.MethodPost, "/api/integrations/connections/"+id+"/test", nil, &out)
	return &out, err
}

// DeleteConnection removes an integration connection by ID.
func (s *Service) DeleteConnection(id string) error {
	return s.c.Do(http.MethodDelete, "/api/integrations/connections/"+id, nil, nil)
}

// ListPlugins returns all available plugins.
func (s *Service) ListPlugins() ([]types.Plugin, error) {
	var out []types.Plugin
	err := s.c.Do(http.MethodGet, "/api/plugins", nil, &out)
	return out, err
}

// InstallPlugin installs a plugin by ID.
func (s *Service) InstallPlugin(id string) (*types.Plugin, error) {
	var out types.Plugin
	err := s.c.Do(http.MethodPost, "/api/plugins/"+id+"/install", nil, &out)
	return &out, err
}
