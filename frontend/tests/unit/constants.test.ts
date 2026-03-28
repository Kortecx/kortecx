import { describe, it, expect } from 'vitest';
import { PROVIDERS, ROLE_META } from '@/lib/constants';
import type { ExpertRole, ProviderSlug } from '@/lib/types';

const VALID_SLUGS: ProviderSlug[] = [
  'anthropic', 'openai', 'google', 'openrouter',
  'mistral', 'cohere', 'together', 'groq',
  'huggingface', 'deepseek', 'xai', 'custom',
];

describe('PROVIDERS', () => {
  it('is a non-empty array', () => {
    expect(Array.isArray(PROVIDERS)).toBe(true);
    expect(PROVIDERS.length).toBeGreaterThan(0);
  });

  it('each provider has required fields', () => {
    for (const p of PROVIDERS) {
      expect(p.id).toBeTruthy();
      expect(p.slug).toBeTruthy();
      expect(p.name).toBeTruthy();
      expect(p.description).toBeTruthy();
      expect(typeof p.color).toBe('string');
      expect(typeof p.connected).toBe('boolean');
      expect(typeof p.apiKeySet).toBe('boolean');
      expect(Array.isArray(p.models)).toBe(true);
    }
  });

  it('each provider has a valid slug', () => {
    for (const p of PROVIDERS) {
      expect(VALID_SLUGS).toContain(p.slug);
    }
  });

  it('has no duplicate provider IDs', () => {
    const ids = PROVIDERS.map(p => p.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('each model has required fields', () => {
    for (const p of PROVIDERS) {
      for (const m of p.models) {
        expect(m.id).toBeTruthy();
        expect(m.name).toBeTruthy();
        expect(m.providerId).toBe(p.id);
        expect(typeof m.contextWindow).toBe('number');
        expect(m.contextWindow).toBeGreaterThan(0);
        expect(typeof m.costInputPer1k).toBe('number');
        expect(typeof m.costOutputPer1k).toBe('number');
        expect(Array.isArray(m.capabilities)).toBe(true);
        expect(m.capabilities.length).toBeGreaterThan(0);
        expect(typeof m.maxOutputTokens).toBe('number');
        expect(typeof m.supportsStreaming).toBe('boolean');
        expect(typeof m.supportsFunctionCalling).toBe('boolean');
      }
    }
  });

  it('has no duplicate model IDs across all providers', () => {
    const ids = PROVIDERS.flatMap(p => p.models.map(m => m.id));
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('every model cost is non-negative', () => {
    for (const p of PROVIDERS) {
      for (const m of p.models) {
        expect(m.costInputPer1k).toBeGreaterThanOrEqual(0);
        expect(m.costOutputPer1k).toBeGreaterThanOrEqual(0);
      }
    }
  });
});

describe('ROLE_META', () => {
  const ALL_ROLES: ExpertRole[] = [
    'researcher', 'analyst', 'writer', 'coder',
    'reviewer', 'planner', 'synthesizer', 'critic',
    'legal', 'financial', 'medical', 'coordinator',
    'data-engineer', 'creative', 'translator', 'custom',
  ];

  it('covers all ExpertRole values', () => {
    for (const role of ALL_ROLES) {
      expect(ROLE_META).toHaveProperty(role);
    }
  });

  it('each role entry has label, emoji, color, dimColor', () => {
    for (const role of ALL_ROLES) {
      const meta = ROLE_META[role];
      expect(typeof meta.label).toBe('string');
      expect(meta.label.length).toBeGreaterThan(0);
      expect(typeof meta.emoji).toBe('string');
      expect(typeof meta.color).toBe('string');
      expect(typeof meta.dimColor).toBe('string');
    }
  });

  it('has no extra roles beyond ExpertRole', () => {
    const keys = Object.keys(ROLE_META);
    for (const key of keys) {
      expect(ALL_ROLES).toContain(key);
    }
  });
});
