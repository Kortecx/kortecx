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
// Pass a *types.ListOptions as the last filter's value to enable pagination.
func (s *Service) ListExperts(opts *types.ListOptions, filters ...Filter) ([]types.Expert, error) {
	path := "/api/experts"
	if q := buildQuery(filters); q != "" {
		path += "?" + q
	}
	path = types.AppendQuery(path, opts)
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

// --- Expert Files ---

// ListExpertFiles returns all files in an expert's directory.
func (s *Service) ListExpertFiles(expertID string) ([]types.ExpertFile, error) {
	var out struct {
		Files []types.ExpertFile `json:"files"`
	}
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/experts/files?expertId=%s", expertID), nil, &out)
	return out.Files, err
}

// UpdateExpertFile updates a single file in an expert's directory (auto-versions).
func (s *Service) UpdateExpertFile(expertID string, req types.UpdateExpertFileRequest) error {
	body := struct {
		types.UpdateExpertFileRequest
		ExpertID string `json:"expertId"`
	}{req, expertID}
	return s.c.Do(http.MethodPost, "/api/experts/files", body, nil)
}

// --- Expert Versions ---

// ListExpertVersions returns version history for a specific file.
func (s *Service) ListExpertVersions(expertID, filename string) ([]types.ExpertVersion, error) {
	var out struct {
		Versions []types.ExpertVersion `json:"versions"`
		Total    int                   `json:"total"`
	}
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/experts/versions?expertId=%s&filename=%s", expertID, filename), nil, &out)
	return out.Versions, err
}

// RestoreExpertVersion restores an expert file to a previous version.
func (s *Service) RestoreExpertVersion(expertID string, req types.RestoreVersionRequest) error {
	body := struct {
		types.RestoreVersionRequest
		ExpertID string `json:"expertId"`
	}{req, expertID}
	return s.c.Do(http.MethodPost, "/api/experts/versions", body, nil)
}

// --- Providers ---

// ListProviders returns all configured providers. Pass optional ListOptions for pagination and sorting.
func (s *Service) ListProviders(opts ...*types.ListOptions) ([]types.Provider, error) {
	path := "/api/providers"
	if len(opts) > 0 {
		path = types.AppendQuery(path, opts[0])
	}
	var out []types.Provider
	err := s.c.Do(http.MethodGet, path, nil, &out)
	return out, err
}

// AddProvider adds a new provider connection.
func (s *Service) AddProvider(req types.CreateProviderRequest) (*types.Provider, error) {
	var out types.Provider
	err := s.c.Do(http.MethodPost, "/api/providers", req, &out)
	return &out, err
}

// UpdateProvider updates an existing provider's configuration.
func (s *Service) UpdateProvider(id string, req types.UpdateProviderRequest) (*types.Provider, error) {
	var out types.Provider
	body := struct {
		types.UpdateProviderRequest
		ID string `json:"id"`
	}{req, id}
	err := s.c.Do(http.MethodPatch, "/api/providers", body, &out)
	return &out, err
}

// DeleteProvider removes a provider by ID.
func (s *Service) DeleteProvider(id string) error {
	return s.c.Do(http.MethodDelete, fmt.Sprintf("/api/providers?id=%s", id), nil, nil)
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
