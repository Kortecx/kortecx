/**
 * Config Export/Import utilities for Kortecx entities.
 * Provides standardized JSON export format with metadata headers
 * and validation for import operations.
 */

export type ExportEntityType = 'expert' | 'workflow' | 'dataset' | 'mcp_server' | 'connection' | 'alert_rule';

export interface KortecxExportConfig {
  _kortecxExport: true;
  _version: string;
  _exportedAt: string;
  _entityType: ExportEntityType;
  [key: string]: unknown;
}

/**
 * Download an entity as a JSON config file.
 */
export function exportToJSON(
  entity: Record<string, unknown>,
  entityType: ExportEntityType,
  filename: string,
): void {
  const config: KortecxExportConfig = {
    _kortecxExport: true,
    _version: '1.0',
    _exportedAt: new Date().toISOString(),
    _entityType: entityType,
    ...entity,
  };
  const blob = new Blob([JSON.stringify(config, null, 2)], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename.endsWith('.json') ? filename : `${filename}.json`;
  a.click();
  URL.revokeObjectURL(url);
}

/**
 * Validate and parse an imported JSON config file.
 * Returns the parsed config or throws an error with a descriptive message.
 */
export function validateImportFile(json: unknown): KortecxExportConfig {
  if (!json || typeof json !== 'object') {
    throw new Error('Invalid file: expected a JSON object');
  }

  const obj = json as Record<string, unknown>;

  if (obj._kortecxExport !== true) {
    throw new Error('Invalid file: not a Kortecx export (missing _kortecxExport flag)');
  }

  if (!obj._entityType || typeof obj._entityType !== 'string') {
    throw new Error('Invalid file: missing _entityType');
  }

  const validTypes: ExportEntityType[] = ['expert', 'workflow', 'dataset', 'mcp_server', 'connection', 'alert_rule'];
  if (!validTypes.includes(obj._entityType as ExportEntityType)) {
    throw new Error(`Invalid file: unknown entity type "${obj._entityType}"`);
  }

  return obj as KortecxExportConfig;
}

/**
 * Read a File object as parsed JSON.
 */
export function readFileAsJSON(file: File): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      try {
        resolve(JSON.parse(reader.result as string));
      } catch {
        reject(new Error('Failed to parse file as JSON'));
      }
    };
    reader.onerror = () => reject(new Error('Failed to read file'));
    reader.readAsText(file);
  });
}

/**
 * Trigger a JSON export by fetching from the export API.
 */
export async function exportEntity(entityType: ExportEntityType, entityId: string, entityName?: string): Promise<void> {
  const res = await fetch(`/api/export?type=${entityType}&id=${encodeURIComponent(entityId)}`);
  if (!res.ok) {
    const err = await res.json().catch(() => ({ error: 'Export failed' }));
    throw new Error(err.error || 'Export failed');
  }
  const data = await res.json();
  const filename = `${entityName || entityType}-${entityId}`.replace(/[^a-zA-Z0-9_-]/g, '_');
  exportToJSON(data, entityType, `${filename}.json`);
}

/**
 * Import a config JSON object via the import API.
 */
export async function importEntity(config: KortecxExportConfig): Promise<{ id: string; name: string; entityType: string }> {
  const res = await fetch('/api/import', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(config),
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({ error: 'Import failed' }));
    throw new Error(err.error || 'Import failed');
  }
  return res.json();
}
