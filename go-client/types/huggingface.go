package types

import "time"

// HuggingFace pipeline tags for inference.
type HFPipeline string

const (
	HFPipelineTextGeneration       HFPipeline = "text-generation"
	HFPipelineText2TextGeneration  HFPipeline = "text2text-generation"
	HFPipelineSummarization        HFPipeline = "summarization"
	HFPipelineTranslation          HFPipeline = "translation"
	HFPipelineFillMask             HFPipeline = "fill-mask"
	HFPipelineQuestionAnswering    HFPipeline = "question-answering"
	HFPipelineSentenceSimilarity   HFPipeline = "sentence-similarity"
	HFPipelineFeatureExtraction    HFPipeline = "feature-extraction"
	HFPipelineTextClassification   HFPipeline = "text-classification"
	HFPipelineTokenClassification  HFPipeline = "token-classification"
	HFPipelineZeroShotClassify     HFPipeline = "zero-shot-classification"
	HFPipelineImageClassification  HFPipeline = "image-classification"
	HFPipelineObjectDetection      HFPipeline = "object-detection"
	HFPipelineImageSegmentation    HFPipeline = "image-segmentation"
	HFPipelineTextToImage          HFPipeline = "text-to-image"
	HFPipelineImageToText          HFPipeline = "image-to-text"
	HFPipelineAutomaticSpeechRecog HFPipeline = "automatic-speech-recognition"
	HFPipelineTextToSpeech         HFPipeline = "text-to-speech"
	HFPipelineConversational       HFPipeline = "conversational"
	HFPipelineTableQuestionAnswer  HFPipeline = "table-question-answering"
)

// HFInferenceStatus tracks inference request lifecycle.
type HFInferenceStatus string

const (
	HFInferenceQueued    HFInferenceStatus = "queued"
	HFInferenceRunning   HFInferenceStatus = "running"
	HFInferenceCompleted HFInferenceStatus = "completed"
	HFInferenceFailed    HFInferenceStatus = "failed"
)

// HFModel represents a model from the HuggingFace Hub.
type HFModel struct {
	ID            string     `json:"id"`
	Author        string     `json:"author,omitempty"`
	ModelID       string     `json:"modelId"`
	SHA           string     `json:"sha,omitempty"`
	Pipeline      HFPipeline `json:"pipeline_tag,omitempty"`
	Tags          []string   `json:"tags,omitempty"`
	Downloads     int        `json:"downloads"`
	Likes         int        `json:"likes"`
	Library       string     `json:"library_name,omitempty"`
	Private       bool       `json:"private"`
	Gated         string     `json:"gated,omitempty"`
	LastModified  time.Time  `json:"lastModified,omitempty"`
	CreatedAt     time.Time  `json:"createdAt,omitempty"`
	InferenceAPI  string     `json:"inference,omitempty"`
	CardData      any        `json:"cardData,omitempty"`
}

// HFDataset represents a dataset from the HuggingFace Hub.
type HFDataset struct {
	ID           string    `json:"id"`
	Author       string    `json:"author,omitempty"`
	SHA          string    `json:"sha,omitempty"`
	Tags         []string  `json:"tags,omitempty"`
	Downloads    int       `json:"downloads"`
	Likes        int       `json:"likes"`
	Private      bool      `json:"private"`
	Gated        string    `json:"gated,omitempty"`
	Description  string    `json:"description,omitempty"`
	Citation     string    `json:"citation,omitempty"`
	LastModified time.Time `json:"lastModified,omitempty"`
	CreatedAt    time.Time `json:"createdAt,omitempty"`
	CardData     any       `json:"cardData,omitempty"`
}

// HFInferenceRequest is sent to run inference on a HuggingFace model.
type HFInferenceRequest struct {
	RequestID  string         `json:"requestId"`
	Model      string         `json:"model"`
	Inputs     any            `json:"inputs"`
	Parameters map[string]any `json:"parameters,omitempty"`
	Options    HFInferOptions `json:"options,omitempty"`
}

// HFInferOptions configures inference behavior.
type HFInferOptions struct {
	UseCache      *bool `json:"use_cache,omitempty"`
	WaitForModel  bool  `json:"wait_for_model,omitempty"`
	UseGPU        bool  `json:"use_gpu,omitempty"`
}

// HFInferenceResponse is the result of a HuggingFace inference call.
type HFInferenceResponse struct {
	RequestID  string            `json:"requestId"`
	Model      string            `json:"model"`
	Status     HFInferenceStatus `json:"status"`
	Output     any               `json:"output,omitempty"`
	Error      string            `json:"error,omitempty"`
	DurationMs int64             `json:"durationMs,omitempty"`
	Timestamp  time.Time         `json:"timestamp"`
}

// --- WebSocket request types for HuggingFace operations ---

// HFSearchModelsRequest is sent over WS to search HuggingFace models.
type HFSearchModelsRequest struct {
	RequestID string     `json:"requestId"`
	Search    string     `json:"search,omitempty"`
	Author    string     `json:"author,omitempty"`
	Pipeline  HFPipeline `json:"pipeline_tag,omitempty"`
	Library   string     `json:"library,omitempty"`
	Sort      string     `json:"sort,omitempty"`
	Direction string     `json:"direction,omitempty"`
	Limit     int        `json:"limit,omitempty"`
	Tags      []string   `json:"tags,omitempty"`
}

// HFSearchDatasetsRequest is sent over WS to search HuggingFace datasets.
type HFSearchDatasetsRequest struct {
	RequestID string   `json:"requestId"`
	Search    string   `json:"search,omitempty"`
	Author    string   `json:"author,omitempty"`
	Sort      string   `json:"sort,omitempty"`
	Direction string   `json:"direction,omitempty"`
	Limit     int      `json:"limit,omitempty"`
	Tags      []string `json:"tags,omitempty"`
}

// HFBatchInferenceRequest sends multiple inference requests to be processed in parallel.
type HFBatchInferenceRequest struct {
	RequestID string               `json:"requestId"`
	Requests  []HFInferenceRequest `json:"requests"`
}

// HFModelDetailRequest retrieves full model info from HuggingFace.
type HFModelDetailRequest struct {
	RequestID string `json:"requestId"`
	ModelID   string `json:"modelId"`
}

// HFDatasetDetailRequest retrieves full dataset info from HuggingFace.
type HFDatasetDetailRequest struct {
	RequestID string `json:"requestId"`
	DatasetID string `json:"datasetId"`
}
