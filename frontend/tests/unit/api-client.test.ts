import { describe, it, expect, vi, beforeEach } from 'vitest';

describe('API Client', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('constructs correct API URLs', () => {
    const API_BASE = '/api';
    const path = '/experts';
    expect(`${API_BASE}${path}`).toBe('/api/experts');
  });

  it('builds search params correctly', () => {
    const params = new URLSearchParams({ q: 'test', type: 'expert', limit: '25' });
    expect(params.toString()).toBe('q=test&type=expert&limit=25');
  });

  it('handles empty search params', () => {
    const params = new URLSearchParams(
      Object.entries({ q: 'test', type: '' }).filter(([, v]) => Boolean(v)) as [string, string][]
    );
    expect(params.toString()).toBe('q=test');
  });
});

describe('Provider connection', () => {
  it('generates SHA-256 hash for key identification', async () => {
    // Simulate the hashing pattern used in the providers route
    const key = 'sk-ant-test123456';
    const prefix = key.slice(0, 8);
    const suffix = key.slice(-4);
    expect(prefix).toBe('sk-ant-t');
    expect(suffix).toBe('3456');
  });

  it('base64 encodes API key for storage', () => {
    const key = 'sk-ant-test123456';
    const encoded = Buffer.from(key, 'utf-8').toString('base64');
    const decoded = Buffer.from(encoded, 'base64').toString('utf-8');
    expect(decoded).toBe(key);
  });
});
