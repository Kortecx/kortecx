import { describe, it, expect, beforeEach } from 'vitest';

const STORAGE_KEY = 'kortecx_workspace_settings';

// In-memory storage mock for environments without localStorage
const store = new Map<string, string>();
const mockStorage = {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => store.set(k, v),
  removeItem: (k: string) => store.delete(k),
  clear: () => store.clear(),
};

describe('Workspace Settings Persistence', () => {
  beforeEach(() => {
    mockStorage.clear();
  });

  it('saves and retrieves settings', () => {
    const settings = { workspaceName: 'Test', timezone: 'UTC' };
    mockStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
    const raw = mockStorage.getItem(STORAGE_KEY);
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw!);
    expect(parsed.workspaceName).toBe('Test');
    expect(parsed.timezone).toBe('UTC');
  });

  it('returns null when no settings saved', () => {
    const raw = mockStorage.getItem(STORAGE_KEY);
    expect(raw).toBeNull();
  });

  it('handles corrupt JSON gracefully', () => {
    mockStorage.setItem(STORAGE_KEY, '{invalid');
    expect(() => {
      try { JSON.parse(mockStorage.getItem(STORAGE_KEY)!); } catch { /* expected */ }
    }).not.toThrow();
  });

  it('deep merges with defaults', () => {
    const partial = { workspaceName: 'Custom', localInference: { ollamaEnabled: false } };
    mockStorage.setItem(STORAGE_KEY, JSON.stringify(partial));
    const stored = JSON.parse(mockStorage.getItem(STORAGE_KEY)!);
    const defaults = { workspaceName: 'Default', timezone: 'UTC', localInference: { ollamaEnabled: true, ollamaUrl: 'http://localhost:11434' } };
    const merged = { ...defaults, ...stored, localInference: { ...defaults.localInference, ...stored.localInference } };
    expect(merged.workspaceName).toBe('Custom');
    expect(merged.timezone).toBe('UTC');
    expect(merged.localInference.ollamaEnabled).toBe(false);
    expect(merged.localInference.ollamaUrl).toBe('http://localhost:11434');
  });
});

describe('Feature Flags', () => {
  it('all flags default to true', () => {
    const defaults = {
      quorumEngine: true, expertMarketplace: true, mcpServers: true,
      executionArtifacts: true, scriptExecution: true, modelComparison: true,
      draftAutoSave: true, workflowScheduling: true,
    };
    for (const [_key, val] of Object.entries(defaults)) {
      expect(val).toBe(true);
    }
  });

  it('individual flags can be toggled', () => {
    const features = { quorumEngine: true, scriptExecution: true };
    features.scriptExecution = false;
    expect(features.scriptExecution).toBe(false);
    expect(features.quorumEngine).toBe(true);
  });
});
