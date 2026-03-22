// Package mcp provides MCP (Model Context Protocol) server management
// for kortecx, including listing, generating, caching, persisting, and testing
// MCP server scripts.
package mcp

import (
	"net/http"

	"github.com/exi/kortecx-go/client"
	"github.com/exi/kortecx-go/types"
)

// Service handles MCP server management operations.
type Service struct {
	c *client.Client
}

// New creates a new MCP service.
func New(c *client.Client) *Service {
	return &Service{c: c}
}

// ListServers returns all MCP servers (prebuilt, persisted, cached).
func (s *Service) ListServers() (*types.McpServersResponse, error) {
	var out types.McpServersResponse
	err := s.c.Do(http.MethodGet, "/api/mcp/servers", nil, &out)
	return &out, err
}

// GetServer returns a specific MCP server by ID.
func (s *Service) GetServer(id string) (*types.McpServer, error) {
	var out types.McpServer
	err := s.c.Do(http.MethodGet, "/api/mcp/servers/"+id, nil, &out)
	return &out, err
}

// Generate creates a new MCP server from a prompt.
func (s *Service) Generate(req types.GenerateMcpRequest) (*types.McpServer, error) {
	var out types.McpServer
	err := s.c.Do(http.MethodPost, "/api/mcp/generate", req, &out)
	return &out, err
}

// Cache saves a generated MCP server to the session cache.
func (s *Service) Cache(req types.CacheMcpRequest) (*types.McpServer, error) {
	var out types.McpServer
	err := s.c.Do(http.MethodPost, "/api/mcp/cache", req, &out)
	return &out, err
}

// Persist saves an MCP server to permanent storage.
func (s *Service) Persist(id string) (*types.McpServer, error) {
	var out types.McpServer
	err := s.c.Do(http.MethodPost, "/api/mcp/persist/"+id, nil, &out)
	return &out, err
}

// Test runs the test suite for an MCP server.
func (s *Service) Test(id string) (*types.McpServer, error) {
	var out types.McpServer
	err := s.c.Do(http.MethodPost, "/api/mcp/test/"+id, nil, &out)
	return &out, err
}
