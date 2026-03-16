// Package huggingface provides HuggingFace Hub and Inference API integration
// for kortecx, including model/dataset discovery and inference execution.
package huggingface

import (
	"fmt"
	"net/http"
	"net/url"
	"strconv"

	"github.com/exi/kortecx-go/client"
	"github.com/exi/kortecx-go/types"
)

// Service handles HuggingFace Hub and Inference API operations.
type Service struct {
	c      *client.Client
	hfKey  string
}

// New creates a new HuggingFace service. The kortecx client is used for
// proxied requests; hfAPIKey is used for direct HuggingFace API calls.
func New(c *client.Client, hfAPIKey string) *Service {
	return &Service{c: c, hfKey: hfAPIKey}
}

// --- Models ---

// SearchModels queries the HuggingFace Hub for models.
func (s *Service) SearchModels(req types.HFSearchModelsRequest) ([]types.HFModel, error) {
	path := s.buildModelSearchPath(req)
	var out []types.HFModel
	err := s.c.Do(http.MethodGet, path, nil, &out)
	return out, err
}

// GetModel returns detailed information about a specific model.
func (s *Service) GetModel(modelID string) (*types.HFModel, error) {
	var out types.HFModel
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/hf/models/%s", url.PathEscape(modelID)), nil, &out)
	return &out, err
}

// --- Datasets ---

// SearchDatasets queries the HuggingFace Hub for datasets.
func (s *Service) SearchDatasets(req types.HFSearchDatasetsRequest) ([]types.HFDataset, error) {
	path := s.buildDatasetSearchPath(req)
	var out []types.HFDataset
	err := s.c.Do(http.MethodGet, path, nil, &out)
	return out, err
}

// GetDataset returns detailed information about a specific dataset.
func (s *Service) GetDataset(datasetID string) (*types.HFDataset, error) {
	var out types.HFDataset
	err := s.c.Do(http.MethodGet, fmt.Sprintf("/api/hf/datasets/%s", url.PathEscape(datasetID)), nil, &out)
	return &out, err
}

// --- Inference ---

// RunInference executes a single inference request against a HuggingFace model.
func (s *Service) RunInference(req types.HFInferenceRequest) (*types.HFInferenceResponse, error) {
	var out types.HFInferenceResponse
	err := s.c.Do(http.MethodPost, "/api/hf/inference", req, &out)
	return &out, err
}

// RunBatchInference executes multiple inference requests. The server processes
// them in parallel and returns all results.
func (s *Service) RunBatchInference(req types.HFBatchInferenceRequest) ([]types.HFInferenceResponse, error) {
	var out []types.HFInferenceResponse
	err := s.c.Do(http.MethodPost, "/api/hf/inference/batch", req, &out)
	return out, err
}

// --- Path builders ---

func (s *Service) buildModelSearchPath(req types.HFSearchModelsRequest) string {
	q := url.Values{}
	if req.Search != "" {
		q.Set("search", req.Search)
	}
	if req.Author != "" {
		q.Set("author", req.Author)
	}
	if req.Pipeline != "" {
		q.Set("pipeline_tag", string(req.Pipeline))
	}
	if req.Library != "" {
		q.Set("library", req.Library)
	}
	if req.Sort != "" {
		q.Set("sort", req.Sort)
	}
	if req.Direction != "" {
		q.Set("direction", req.Direction)
	}
	if req.Limit > 0 {
		q.Set("limit", strconv.Itoa(req.Limit))
	}
	for _, tag := range req.Tags {
		q.Add("tags", tag)
	}
	path := "/api/hf/models"
	if encoded := q.Encode(); encoded != "" {
		path += "?" + encoded
	}
	return path
}

func (s *Service) buildDatasetSearchPath(req types.HFSearchDatasetsRequest) string {
	q := url.Values{}
	if req.Search != "" {
		q.Set("search", req.Search)
	}
	if req.Author != "" {
		q.Set("author", req.Author)
	}
	if req.Sort != "" {
		q.Set("sort", req.Sort)
	}
	if req.Direction != "" {
		q.Set("direction", req.Direction)
	}
	if req.Limit > 0 {
		q.Set("limit", strconv.Itoa(req.Limit))
	}
	for _, tag := range req.Tags {
		q.Add("tags", tag)
	}
	path := "/api/hf/datasets"
	if encoded := q.Encode(); encoded != "" {
		path += "?" + encoded
	}
	return path
}
