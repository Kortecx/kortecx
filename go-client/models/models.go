// Package models provides model and expert management for kortecx,
// including expert deployment, provider configuration, and model discovery.
package models

import (
	"fmt"
	"net/http"

	"github.com/exi/kortecx-go/client"
	"github.com/exi/kortecx-go/types"
)

// Service handles model and expert management operations.
type Service struct {
	c *client.Client
}

// New creates a new models service.
func New(c *client.Client) *Service {
	return &Service{c: c}
}

// --- Experts ---

// ListExperts returns all available experts. Optional filters: role, status, search query.
func (s *Service) ListExperts(filters ...Filter) ([]types.Expert, error) {
	path := "/api/experts"
	if q := buildQuery(filters); q != "" {
		path += "?" + q
	}
	var out []types.Expert
	err := s.c.Do(http.MethodGet, path, nil, &out)
	return out, err
}

// GetExpert returns an expert by ID.
func (s *Service) GetExpert(id string) (*types.Expert, error) {
	var out types.Expert
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/experts?id=%s", id), nil, &out)
	return &out, err
}

// DeployExpert deploys a new expert with the given configuration.
func (s *Service) DeployExpert(req types.DeployExpertRequest) (*types.Expert, error) {
	var out types.Expert
	err := s.c.Do(http.MethodPost, "/api/experts", req, &out)
	return &out, err
}

// UpdateExpert updates an existing expert's configuration.
func (s *Service) UpdateExpert(id string, req types.UpdateExpertRequest) (*types.Expert, error) {
	var out types.Expert
	body := struct {
		types.UpdateExpertRequest
		ID string `json:"id"`
	}{req, id}
	err := s.c.Do(http.MethodPatch, "/api/experts", body, &out)
	return &out, err
}

// DeleteExpert removes an expert by ID.
func (s *Service) DeleteExpert(id string) error {
	return s.c.Do(http.MethodDelete, fmt.Sprintf("/api/experts?id=%s", id), nil, nil)
}

// --- Providers ---

// ListProviders returns all configured providers.
func (s *Service) ListProviders() ([]types.Provider, error) {
	var out []types.Provider
	err := s.c.Do(http.MethodGet, "/api/providers", nil, &out)
	return out, err
}

// AddProvider adds a new provider connection.
func (s *Service) AddProvider(req types.CreateProviderRequest) (*types.Provider, error) {
	var out types.Provider
	err := s.c.Do(http.MethodPost, "/api/providers", req, &out)
	return &out, err
}

// --- Filters ---

// Filter is a query parameter for list operations.
type Filter struct {
	Key   string
	Value string
}

// WithRole filters by expert role.
func WithRole(role types.ExpertRole) Filter {
	return Filter{Key: "role", Value: string(role)}
}

// WithStatus filters by status.
func WithStatus(status string) Filter {
	return Filter{Key: "status", Value: status}
}

// WithSearch filters by search query.
func WithSearch(query string) Filter {
	return Filter{Key: "search", Value: query}
}

func buildQuery(filters []Filter) string {
	if len(filters) == 0 {
		return ""
	}
	q := ""
	for i, f := range filters {
		if i > 0 {
			q += "&"
		}
		q += f.Key + "=" + f.Value
	}
	return q
}
