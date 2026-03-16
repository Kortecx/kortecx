package client

import (
	"testing"
)

func TestNewClient(t *testing.T) {
	c := New("http://localhost:8000")
	if c == nil {
		t.Fatal("expected non-nil client")
	}
	if c.BaseURL != "http://localhost:8000" {
		t.Errorf("expected BaseURL http://localhost:8000, got %s", c.BaseURL)
	}
}

func TestNewClientWithAPIKey(t *testing.T) {
	c := New("http://localhost:8000", WithAPIKey("test-key"))
	if c.apiKey != "test-key" {
		t.Errorf("expected apiKey test-key, got %s", c.apiKey)
	}
}

func TestWebSocketURL(t *testing.T) {
	tests := []struct {
		input string
		want  string
	}{
		{"http://localhost:8000", "ws://localhost:8000/ws"},
		{"https://api.example.com", "wss://api.example.com/ws"},
		{"http://localhost:8000/", "ws://localhost:8000/ws"},
	}
	for _, tt := range tests {
		c := New(tt.input)
		got, err := c.WebSocketURL()
		if err != nil {
			t.Errorf("New(%q).WebSocketURL() error: %v", tt.input, err)
			continue
		}
		if got != tt.want {
			t.Errorf("New(%q).WebSocketURL() = %q, want %q", tt.input, got, tt.want)
		}
	}
}
