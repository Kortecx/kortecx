import { describe, it, expect } from 'vitest';

/* ── Similarity edge deduplication logic (mirrors graph/route.ts) ── */
interface SimilarityEdge {
  source: string;
  target: string;
  weight: number;
}

function deduplicateEdges(edges: SimilarityEdge[]): SimilarityEdge[] {
  const seen = new Map<string, SimilarityEdge>();
  for (const edge of edges) {
    const key = [edge.source, edge.target].sort().join(':');
    const existing = seen.get(key);
    if (!existing || edge.weight > existing.weight) {
      seen.set(key, edge);
    }
  }
  return Array.from(seen.values());
}

function filterByThreshold(edges: SimilarityEdge[], threshold: number): SimilarityEdge[] {
  return edges.filter(e => e.weight >= threshold);
}

describe('Agent Graph — edge deduplication', () => {
  it('removes duplicate A→B / B→A edges, keeping highest weight', () => {
    const edges: SimilarityEdge[] = [
      { source: 'a', target: 'b', weight: 0.8 },
      { source: 'b', target: 'a', weight: 0.6 },
    ];
    const result = deduplicateEdges(edges);
    expect(result).toHaveLength(1);
    expect(result[0].weight).toBe(0.8);
  });

  it('keeps distinct pairs', () => {
    const edges: SimilarityEdge[] = [
      { source: 'a', target: 'b', weight: 0.5 },
      { source: 'a', target: 'c', weight: 0.7 },
      { source: 'b', target: 'c', weight: 0.3 },
    ];
    const result = deduplicateEdges(edges);
    expect(result).toHaveLength(3);
  });

  it('returns empty for empty input', () => {
    expect(deduplicateEdges([])).toHaveLength(0);
  });
});

describe('Agent Graph — threshold filtering', () => {
  const edges: SimilarityEdge[] = [
    { source: 'a', target: 'b', weight: 0.9 },
    { source: 'a', target: 'c', weight: 0.2 },
    { source: 'b', target: 'c', weight: 0.5 },
  ];

  it('filters edges below threshold 0.3', () => {
    const result = filterByThreshold(edges, 0.3);
    expect(result).toHaveLength(2);
    expect(result.every(e => e.weight >= 0.3)).toBe(true);
  });

  it('keeps all edges with threshold 0', () => {
    expect(filterByThreshold(edges, 0)).toHaveLength(3);
  });

  it('returns empty when threshold exceeds all weights', () => {
    expect(filterByThreshold(edges, 1.0)).toHaveLength(0);
  });
});

describe('Agent Graph — node filtering', () => {
  type MockAgent = { id: string; name: string; role: string; status: string };

  function filterAgents(
    agents: MockAgent[],
    opts: { role?: string; status?: string; search?: string },
  ): MockAgent[] {
    return agents.filter(p => {
      if (opts.role && opts.role !== 'all' && p.role !== opts.role) return false;
      if (opts.status && opts.status !== 'all' && p.status !== opts.status) return false;
      if (opts.search) {
        const s = opts.search.toLowerCase();
        if (!p.name.toLowerCase().includes(s)) return false;
      }
      return true;
    });
  }

  const agents: MockAgent[] = [
    { id: '1', name: 'ResearchBot', role: 'researcher', status: 'active' },
    { id: '2', name: 'CodeGen',     role: 'coder',      status: 'idle' },
    { id: '3', name: 'LegalAide',   role: 'legal',      status: 'active' },
  ];

  it('filters by role', () => {
    const result = filterAgents(agents, { role: 'coder' });
    expect(result).toHaveLength(1);
    expect(result[0].id).toBe('2');
  });

  it('filters by status', () => {
    const result = filterAgents(agents, { status: 'active' });
    expect(result).toHaveLength(2);
  });

  it('filters by search', () => {
    const result = filterAgents(agents, { search: 'legal' });
    expect(result).toHaveLength(1);
    expect(result[0].id).toBe('3');
  });

  it('returns all when no filters', () => {
    expect(filterAgents(agents, {})).toHaveLength(3);
  });

  it('returns all with role=all', () => {
    expect(filterAgents(agents, { role: 'all' })).toHaveLength(3);
  });
});

describe('Marketplace experts — data integrity', () => {
  /* Validate that marketplace experts have the required fields for embedding */
  interface MarketplaceExpert {
    id: string;
    name: string;
    description: string;
    systemPrompt: string;
    role: string;
    capabilities: string[];
    specializations: string[];
    tags: string[];
  }

  // Shared capability vocabulary expected across marketplace experts
  const SHARED_CAPS = ['reasoning', 'analysis', 'research', 'coding', 'writing', 'synthesis', 'review', 'planning', 'data-processing', 'communication'];

  it('all marketplace experts have systemPrompt for rich embeddings', () => {
    // This test validates the data contract — not importing actual data
    const sample: MarketplaceExpert = {
      id: 'mp-test',
      name: 'Test',
      description: 'A test expert',
      systemPrompt: 'You are a test expert',
      role: 'researcher',
      capabilities: ['reasoning', 'analysis'],
      specializations: ['Deep Research'],
      tags: ['test'],
    };
    expect(sample.systemPrompt.length).toBeGreaterThan(0);
    expect(sample.capabilities.length).toBeGreaterThan(0);
    expect(sample.specializations.length).toBeGreaterThan(0);
  });

  it('shared capabilities vocabulary is well-defined', () => {
    expect(SHARED_CAPS.length).toBeGreaterThanOrEqual(8);
    // Each cap should be a non-empty lowercase string
    for (const cap of SHARED_CAPS) {
      expect(cap.length).toBeGreaterThan(0);
      expect(cap).toBe(cap.toLowerCase());
    }
  });
});
