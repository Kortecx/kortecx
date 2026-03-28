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

describe('PRISM Graph — edge deduplication', () => {
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

describe('PRISM Graph — threshold filtering', () => {
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

describe('PRISM Graph — node filtering', () => {
  type MockPrism = { id: string; name: string; role: string; status: string };

  function filterPrisms(
    prisms: MockPrism[],
    opts: { role?: string; status?: string; search?: string },
  ): MockPrism[] {
    return prisms.filter(p => {
      if (opts.role && opts.role !== 'all' && p.role !== opts.role) return false;
      if (opts.status && opts.status !== 'all' && p.status !== opts.status) return false;
      if (opts.search) {
        const s = opts.search.toLowerCase();
        if (!p.name.toLowerCase().includes(s)) return false;
      }
      return true;
    });
  }

  const prisms: MockPrism[] = [
    { id: '1', name: 'ResearchBot', role: 'researcher', status: 'active' },
    { id: '2', name: 'CodeGen',     role: 'coder',      status: 'idle' },
    { id: '3', name: 'LegalAide',   role: 'legal',      status: 'active' },
  ];

  it('filters by role', () => {
    const result = filterPrisms(prisms, { role: 'coder' });
    expect(result).toHaveLength(1);
    expect(result[0].id).toBe('2');
  });

  it('filters by status', () => {
    const result = filterPrisms(prisms, { status: 'active' });
    expect(result).toHaveLength(2);
  });

  it('filters by search', () => {
    const result = filterPrisms(prisms, { search: 'legal' });
    expect(result).toHaveLength(1);
    expect(result[0].id).toBe('3');
  });

  it('returns all when no filters', () => {
    expect(filterPrisms(prisms, {})).toHaveLength(3);
  });

  it('returns all with role=all', () => {
    expect(filterPrisms(prisms, { role: 'all' })).toHaveLength(3);
  });
});
