package client

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"time"
)

// Client is the base HTTP client for communicating with kortecx APIs.
type Client struct {
	BaseURL    string
	HTTPClient *http.Client
	apiKey     string
}

// Option configures the Client.
type Option func(*Client)

// WithAPIKey sets the API key for authentication.
func WithAPIKey(key string) Option {
	return func(c *Client) { c.apiKey = key }
}

// WithHTTPClient sets a custom http.Client.
func WithHTTPClient(hc *http.Client) Option {
	return func(c *Client) { c.HTTPClient = hc }
}

// WithTimeout sets the default request timeout.
func WithTimeout(d time.Duration) Option {
	return func(c *Client) { c.HTTPClient.Timeout = d }
}

// New creates a new Client targeting the given kortecx base URL.
func New(baseURL string, opts ...Option) *Client {
	c := &Client{
		BaseURL: baseURL,
		HTTPClient: &http.Client{
			Timeout: 30 * time.Second,
		},
	}
	for _, o := range opts {
		o(c)
	}
	return c
}

// Do executes an HTTP request and decodes the JSON response.
// It uses context.Background() internally. For explicit context control, use DoCtx.
func (c *Client) Do(method, path string, body any, out any) error {
	return c.DoCtx(context.Background(), method, path, body, out)
}

// DoCtx executes an HTTP request with the given context and decodes the JSON response.
func (c *Client) DoCtx(ctx context.Context, method, path string, body any, out any) error {
	u, err := url.JoinPath(c.BaseURL, path)
	if err != nil {
		return fmt.Errorf("building URL: %w", err)
	}

	var reqBody io.Reader
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return fmt.Errorf("marshaling request: %w", err)
		}
		reqBody = bytes.NewReader(b)
	}

	req, err := http.NewRequestWithContext(ctx, method, u, reqBody)
	if err != nil {
		return fmt.Errorf("creating request: %w", err)
	}

	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json")
	if c.apiKey != "" {
		req.Header.Set("Authorization", "Bearer "+c.apiKey)
	}

	resp, err := c.HTTPClient.Do(req)
	if err != nil {
		return fmt.Errorf("executing request: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		respBody, _ := io.ReadAll(resp.Body)
		return &APIError{
			StatusCode: resp.StatusCode,
			Body:       string(respBody),
		}
	}

	if out != nil {
		if err := json.NewDecoder(resp.Body).Decode(out); err != nil {
			return fmt.Errorf("decoding response: %w", err)
		}
	}

	return nil
}

// WebSocketURL returns the WebSocket URL derived from the base URL.
func (c *Client) WebSocketURL() (string, error) {
	u, err := url.Parse(c.BaseURL)
	if err != nil {
		return "", err
	}
	switch u.Scheme {
	case "https":
		u.Scheme = "wss"
	default:
		u.Scheme = "ws"
	}
	u.Path = "/ws"
	return u.String(), nil
}

// APIError represents an HTTP error from the kortecx API.
type APIError struct {
	StatusCode int
	Body       string
}

func (e *APIError) Error() string {
	return fmt.Sprintf("kortecx API error (status %d): %s", e.StatusCode, e.Body)
}
