// Package data provides data management operations for kortecx,
// including datasets and training job management.
package data

import (
	"fmt"
	"net/http"

	"github.com/exi/kortecx-go/client"
	"github.com/exi/kortecx-go/types"
)

// Service handles data management operations.
type Service struct {
	c *client.Client
}

// New creates a new data management service.
func New(c *client.Client) *Service {
	return &Service{c: c}
}

// --- Datasets ---

// ListDatasets returns all datasets.
func (s *Service) ListDatasets() ([]types.Dataset, error) {
	var out []types.Dataset
	err := s.c.Do(http.MethodGet, "/api/training", nil, &out)
	return out, err
}

// GetDataset returns a dataset by ID.
func (s *Service) GetDataset(id string) (*types.Dataset, error) {
	var out types.Dataset
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/training?datasetId=%s", id), nil, &out)
	return &out, err
}

// CreateDataset creates a new dataset.
func (s *Service) CreateDataset(req types.CreateDatasetRequest) (*types.Dataset, error) {
	var out types.Dataset
	err := s.c.Do(http.MethodPost, "/api/training", req, &out)
	return &out, err
}

// --- Training Jobs ---

// ListTrainingJobs returns all training jobs.
func (s *Service) ListTrainingJobs() ([]types.TrainingJob, error) {
	var out []types.TrainingJob
	err := s.c.Do(http.MethodGet, "/api/training?type=jobs", nil, &out)
	return out, err
}

// GetTrainingJob returns a training job by ID.
func (s *Service) GetTrainingJob(id string) (*types.TrainingJob, error) {
	var out types.TrainingJob
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/training?jobId=%s", id), nil, &out)
	return &out, err
}

// StartTraining starts a new training job.
func (s *Service) StartTraining(req types.StartTrainingRequest) (*types.TrainingJob, error) {
	var out types.TrainingJob
	err := s.c.Do(http.MethodPost, "/api/training", req, &out)
	return &out, err
}

// --- Analytics ---

// GetAnalytics returns weekly analytics data.
func (s *Service) GetAnalytics() (*types.Analytics, error) {
	var out types.Analytics
	err := s.c.Do(http.MethodGet, "/api/analytics", nil, &out)
	return &out, err
}

// GetMetrics returns current system metrics snapshot.
func (s *Service) GetMetrics() (*types.Metrics, error) {
	var out types.Metrics
	err := s.c.Do(http.MethodGet, "/api/metrics", nil, &out)
	return &out, err
}
