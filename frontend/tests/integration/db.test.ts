import { describe, it, expect } from 'vitest';

describe('Database schema validation', () => {
  it('schema module exports all expected tables', async () => {
    const schema = await import('@/lib/db/schema');
    const expectedTables = [
      'metrics', 'tasks', 'workflowRuns', 'alerts', 'logs',
      'experts', 'workflows', 'workflowSteps',
      'datasets', 'hfDatasets', 'integrations', 'integrationConnections',
      'plugins', 'projects', 'apiKeys', 'synthesisJobs',
    ];
    for (const table of expectedTables) {
      expect(schema).toHaveProperty(table);
    }
  });

  it('table objects are valid Drizzle pgTable instances', async () => {
    const schema = await import('@/lib/db/schema');
    // Drizzle pgTable objects have a Symbol-based internal structure
    // Verify they're objects with expected column-like properties
    const tables = [schema.experts, schema.workflows, schema.synthesisJobs, schema.apiKeys, schema.hfDatasets];
    for (const table of tables) {
      expect(typeof table).toBe('object');
      expect(table).not.toBeNull();
      // All tables should have an id column accessor
      expect(table).toHaveProperty('id');
    }
  });
});
