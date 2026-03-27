import { describe, it, expect } from 'vitest';

describe('PRISM Bundle — payload construction', () => {
  function buildPayload(form: {
    name: string;
    role: string;
    description: string;
    category: string;
    tags: string;
    complexityLevel: number;
    capabilities: string[];
    specializations: string;
  }) {
    return {
      name: form.name.trim(),
      role: form.role,
      description: form.description.trim(),
      category: form.category,
      complexityLevel: form.complexityLevel,
      tags: form.tags.split(',').map(t => t.trim()).filter(Boolean),
      capabilities: form.capabilities,
      specializations: form.specializations.split(',').map(s => s.trim()).filter(Boolean),
    };
  }

  it('builds payload with all fields', () => {
    const payload = buildPayload({
      name: '  ResearchBot  ',
      role: 'researcher',
      description: 'Deep research',
      category: 'research',
      tags: 'nlp, rag, research',
      complexityLevel: 4,
      capabilities: ['web-search', 'reasoning'],
      specializations: 'NLP, summarisation',
    });

    expect(payload.name).toBe('ResearchBot');
    expect(payload.category).toBe('research');
    expect(payload.tags).toEqual(['nlp', 'rag', 'research']);
    expect(payload.complexityLevel).toBe(4);
    expect(payload.capabilities).toEqual(['web-search', 'reasoning']);
    expect(payload.specializations).toEqual(['NLP', 'summarisation']);
  });

  it('handles empty optional fields', () => {
    const payload = buildPayload({
      name: 'MinimalBot',
      role: 'custom',
      description: '',
      category: 'custom',
      tags: '',
      complexityLevel: 3,
      capabilities: [],
      specializations: '',
    });

    expect(payload.tags).toEqual([]);
    expect(payload.capabilities).toEqual([]);
    expect(payload.specializations).toEqual([]);
    expect(payload.complexityLevel).toBe(3);
  });

  it('filters empty tags from comma-separated string', () => {
    const payload = buildPayload({
      name: 'Test',
      role: 'coder',
      description: '',
      category: 'engineering',
      tags: 'a, , b, , c',
      complexityLevel: 2,
      capabilities: [],
      specializations: '',
    });

    expect(payload.tags).toEqual(['a', 'b', 'c']);
  });
});

describe('PRISM Bundle — validation', () => {
  function validate(name: string, role: string): Record<string, string> {
    const errors: Record<string, string> = {};
    if (!name.trim()) errors.name = 'PRISM name is required';
    else if (name.trim().length < 2) errors.name = 'Name must be at least 2 characters';
    if (!role) errors.role = 'Role is required';
    return errors;
  }

  it('passes with valid inputs', () => {
    expect(validate('ResearchBot', 'researcher')).toEqual({});
  });

  it('fails with empty name', () => {
    const errors = validate('', 'researcher');
    expect(errors.name).toBe('PRISM name is required');
  });

  it('fails with short name', () => {
    const errors = validate('A', 'researcher');
    expect(errors.name).toBe('Name must be at least 2 characters');
  });

  it('fails with whitespace-only name', () => {
    const errors = validate('   ', 'researcher');
    expect(errors.name).toBe('PRISM name is required');
  });
});

describe('PRISM Bundle — embedding text construction', () => {
  function buildEmbeddingText(expert: {
    name: string;
    description: string;
    role: string;
    category: string;
    tags: string[];
    capabilities: string[];
  }): string {
    return [
      expert.name,
      expert.description,
      `Role: ${expert.role}`,
      `Category: ${expert.category}`,
      `Tags: ${expert.tags.join(', ')}`,
      `Capabilities: ${expert.capabilities.join(', ')}`,
    ].filter(Boolean).join('. ');
  }

  it('constructs embedding text from metadata', () => {
    const text = buildEmbeddingText({
      name: 'ResearchBot',
      description: 'Deep research',
      role: 'researcher',
      category: 'research',
      tags: ['nlp', 'rag'],
      capabilities: ['web-search'],
    });

    expect(text).toContain('ResearchBot');
    expect(text).toContain('Deep research');
    expect(text).toContain('Role: researcher');
    expect(text).toContain('Category: research');
    expect(text).toContain('Tags: nlp, rag');
    expect(text).toContain('Capabilities: web-search');
  });

  it('handles empty arrays', () => {
    const text = buildEmbeddingText({
      name: 'Bot',
      description: '',
      role: 'custom',
      category: 'custom',
      tags: [],
      capabilities: [],
    });

    expect(text).toContain('Bot');
    expect(text).toContain('Tags: ');
  });
});
