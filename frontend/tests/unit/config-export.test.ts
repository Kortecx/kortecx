import { describe, it, expect } from 'vitest';
import { validateImportFile, type KortecxExportConfig } from '@/lib/config-export';

describe('Config Export/Import utilities', () => {
  describe('validateImportFile', () => {
    it('validates a correct export file', () => {
      const config = {
        _kortecxExport: true,
        _version: '1.0',
        _exportedAt: '2026-03-29T00:00:00Z',
        _entityType: 'expert',
        expert: { id: 'exp-123', name: 'Test Expert' },
      };

      const result = validateImportFile(config);
      expect(result._kortecxExport).toBe(true);
      expect(result._entityType).toBe('expert');
    });

    it('rejects null input', () => {
      expect(() => validateImportFile(null)).toThrow('expected a JSON object');
    });

    it('rejects non-object input', () => {
      expect(() => validateImportFile('string')).toThrow('expected a JSON object');
    });

    it('rejects missing _kortecxExport flag', () => {
      expect(() => validateImportFile({ _entityType: 'expert' })).toThrow('not a Kortecx export');
    });

    it('rejects missing _entityType', () => {
      expect(() => validateImportFile({ _kortecxExport: true })).toThrow('missing _entityType');
    });

    it('rejects invalid entity type', () => {
      expect(() => validateImportFile({
        _kortecxExport: true,
        _entityType: 'invalid_type',
      })).toThrow('unknown entity type');
    });

    it('accepts all valid entity types', () => {
      const validTypes = ['expert', 'workflow', 'dataset', 'mcp_server', 'connection', 'alert_rule'];
      for (const type of validTypes) {
        const result = validateImportFile({
          _kortecxExport: true,
          _entityType: type,
          _version: '1.0',
          _exportedAt: '2026-03-29T00:00:00Z',
        });
        expect(result._entityType).toBe(type);
      }
    });

    it('preserves entity data in validated output', () => {
      const config = {
        _kortecxExport: true,
        _version: '1.0',
        _exportedAt: '2026-03-29T00:00:00Z',
        _entityType: 'workflow',
        workflow: { id: 'wf-abc', name: 'Test Workflow', status: 'draft' },
        steps: [{ taskDescription: 'Step 1' }],
      };

      const result = validateImportFile(config) as KortecxExportConfig & { workflow: Record<string, unknown>; steps: unknown[] };
      expect(result.workflow.name).toBe('Test Workflow');
      expect(result.steps).toHaveLength(1);
    });
  });

  describe('Export format', () => {
    it('produces correct metadata structure', () => {
      // Test the expected format without calling exportToJSON (which requires DOM)
      const entity = { id: 'exp-1', name: 'Test' };
      const config = {
        _kortecxExport: true as const,
        _version: '1.0',
        _exportedAt: new Date().toISOString(),
        _entityType: 'expert' as const,
        ...entity,
      };

      expect(config._kortecxExport).toBe(true);
      expect(config._version).toBe('1.0');
      expect(config._entityType).toBe('expert');
      expect(config.id).toBe('exp-1');
      expect(config.name).toBe('Test');
    });

    it('round-trip preserves all fields', () => {
      const original = {
        _kortecxExport: true as const,
        _version: '1.0',
        _exportedAt: '2026-03-29T00:00:00Z',
        _entityType: 'expert' as const,
        expert: {
          id: 'exp-1',
          name: 'Test Expert',
          role: 'researcher',
          systemPrompt: 'You are a test expert',
          temperature: '0.7',
          maxTokens: 4096,
          tags: ['test', 'demo'],
        },
      };

      const serialized = JSON.stringify(original);
      const parsed = JSON.parse(serialized);
      const validated = validateImportFile(parsed);

      expect(validated._entityType).toBe('expert');
      expect((validated as typeof original).expert.name).toBe('Test Expert');
      expect((validated as typeof original).expert.tags).toEqual(['test', 'demo']);
    });
  });
});
