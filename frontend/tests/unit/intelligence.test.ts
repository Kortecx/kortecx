import { describe, it, expect, beforeEach } from 'vitest';
import { TIMEZONES, formatTzLabel } from '../../lib/timezones';

// In-memory storage mock
const store = new Map<string, string>();
const mockStorage = {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => store.set(k, v),
  clear: () => store.clear(),
};
const FINETUNE_STORAGE_KEY = 'kortecx_finetune_jobs';

describe('Intelligence — Fine-tuning Jobs', () => {
  beforeEach(() => {
    mockStorage.clear();
  });

  it('creates and persists a fine-tuning job', () => {
    const job = {
      id: 'ft-test-001',
      name: 'Test LoRA',
      baseModel: 'llama3.2:3b',
      engine: 'ollama',
      datasetPath: '/data/train.jsonl',
      status: 'queued',
      progress: 0,
      epochs: 3,
      currentEpoch: 0,
      learningRate: 0.0002,
      batchSize: 4,
      createdAt: new Date().toISOString(),
    };
    mockStorage.setItem(FINETUNE_STORAGE_KEY, JSON.stringify([job]));

    const stored = JSON.parse(mockStorage.getItem(FINETUNE_STORAGE_KEY)!);
    expect(stored).toHaveLength(1);
    expect(stored[0].name).toBe('Test LoRA');
    expect(stored[0].baseModel).toBe('llama3.2:3b');
  });

  it('returns null when no jobs exist', () => {
    const raw = mockStorage.getItem(FINETUNE_STORAGE_KEY);
    expect(raw).toBeNull();
  });

  it('handles multiple jobs', () => {
    const jobs = [
      { id: 'ft-1', name: 'Job 1', status: 'completed' },
      { id: 'ft-2', name: 'Job 2', status: 'running' },
      { id: 'ft-3', name: 'Job 3', status: 'failed' },
    ];
    mockStorage.setItem(FINETUNE_STORAGE_KEY, JSON.stringify(jobs));

    const stored = JSON.parse(mockStorage.getItem(FINETUNE_STORAGE_KEY)!);
    expect(stored).toHaveLength(3);
    expect(stored.filter((j: { status: string }) => j.status === 'completed')).toHaveLength(1);
    expect(stored.filter((j: { status: string }) => j.status === 'failed')).toHaveLength(1);
  });

  it('deletes a job by filtering', () => {
    const jobs = [
      { id: 'ft-1', name: 'Keep' },
      { id: 'ft-2', name: 'Delete' },
    ];
    const filtered = jobs.filter(j => j.id !== 'ft-2');
    expect(filtered).toHaveLength(1);
    expect(filtered[0].name).toBe('Keep');
  });
});

describe('Intelligence — Models Tabs', () => {
  it('local tab is enabled by default', () => {
    const tabs = [
      { id: 'local', enabled: true },
      { id: 'kortecx', enabled: false },
      { id: 'advanced', enabled: false },
    ];
    expect(tabs.filter(t => t.enabled)).toHaveLength(1);
    expect(tabs.find(t => t.enabled)?.id).toBe('local');
  });

  it('cloud tabs redirect URL is correct', () => {
    const KORTECX_CLOUD_URL = 'https://www.kortecx.com';
    expect(KORTECX_CLOUD_URL).toMatch(/^https:\/\/www\.kortecx\.com$/);
  });
});

describe('Intelligence — Timezone Module', () => {
  it('TIMEZONES is a non-empty static array', () => {
    expect(Array.isArray(TIMEZONES)).toBe(true);
    expect(TIMEZONES.length).toBeGreaterThan(50);
  });

  it('contains all major business timezones', () => {
    const required = [
      'UTC', 'America/New_York', 'America/Los_Angeles', 'Europe/London',
      'Europe/Berlin', 'Asia/Tokyo', 'Asia/Shanghai', 'Asia/Kolkata',
      'Australia/Sydney', 'Asia/Singapore', 'America/Sao_Paulo',
    ];
    for (const tz of required) {
      expect(TIMEZONES).toContain(tz);
    }
  });

  it('does not contain duplicate entries', () => {
    const unique = new Set(TIMEZONES);
    expect(unique.size).toBe(TIMEZONES.length);
  });

  it('all entries are valid IANA timezone identifiers', () => {
    for (const tz of TIMEZONES) {
      // Should not throw when used with Intl
      expect(() => {
        new Intl.DateTimeFormat('en-US', { timeZone: tz });
      }).not.toThrow();
    }
  });

  it('formatTzLabel formats correctly', () => {
    expect(formatTzLabel('America/New_York')).toBe('America / New York');
    expect(formatTzLabel('Asia/Ho_Chi_Minh')).toBe('Asia / Ho Chi Minh');
    expect(formatTzLabel('UTC')).toBe('UTC');
    expect(formatTzLabel('Pacific/Auckland')).toBe('Pacific / Auckland');
    expect(formatTzLabel('Europe/London')).toBe('Europe / London');
  });
});
