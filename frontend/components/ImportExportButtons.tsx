'use client';

import { useRef, useState } from 'react';
import { Download, Upload, Loader2, FolderOpen } from 'lucide-react';
import { motion } from 'framer-motion';
import { buttonHover } from '@/lib/motion';
import type { ExportEntityType } from '@/lib/config-export';
import { exportEntity, readFileAsJSON, validateImportFile, importEntity } from '@/lib/config-export';

/* ── Export Button ── */
interface ExportButtonProps {
  entityType: ExportEntityType;
  entityId: string;
  entityName?: string;
  size?: 'sm' | 'md';
}

export function ExportButton({ entityType, entityId, entityName, size = 'sm' }: ExportButtonProps) {
  const [exporting, setExporting] = useState(false);

  const handleExport = async () => {
    setExporting(true);
    try {
      await exportEntity(entityType, entityId, entityName);
    } catch (err) {
      console.error('Export failed:', err);
    }
    setExporting(false);
  };

  return (
    <motion.button
      {...buttonHover}
      className={`btn btn-secondary btn-${size}`}
      onClick={handleExport}
      disabled={exporting}
      style={{ display: 'flex', alignItems: 'center', gap: 4 }}
      title="Export as JSON"
    >
      {exporting ? <Loader2 size={12} style={{ animation: 'spin 1s linear infinite' }} /> : <Download size={12} />}
      {size === 'md' && (exporting ? 'Exporting...' : 'Export')}
    </motion.button>
  );
}

/* ── Import Button ── */
interface ImportButtonProps {
  entityType?: ExportEntityType;
  onImported: (result: { id: string; name: string; entityType: string }) => void;
  size?: 'sm' | 'md';
  label?: string;
}

export function ImportButton({ entityType, onImported, size = 'sm', label }: ImportButtonProps) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleFile = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;

    setImporting(true);
    setError(null);

    try {
      const json = await readFileAsJSON(file);
      const config = validateImportFile(json);

      // If entityType is specified, validate it matches
      if (entityType && config._entityType !== entityType) {
        throw new Error(`Expected ${entityType} config but got ${config._entityType}`);
      }

      const result = await importEntity(config);
      onImported(result);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Import failed';
      setError(msg);
      console.error('Import failed:', msg);
    }
    setImporting(false);

    // Reset file input
    if (fileInputRef.current) fileInputRef.current.value = '';
  };

  return (
    <div style={{ position: 'relative' }}>
      <motion.button
        {...buttonHover}
        className={`btn btn-secondary btn-${size}`}
        onClick={() => fileInputRef.current?.click()}
        disabled={importing}
        style={{
          display: 'flex', alignItems: 'center', gap: 6,
          ...(size === 'md' ? { padding: '8px 15px', fontSize: 12, borderRadius: 8 } : {}),
        }}
        title={error || 'Import from JSON file'}
      >
        {importing ? <Loader2 size={12} style={{ animation: 'spin 1s linear infinite' }} /> : <Upload size={12} />}
        {(size === 'md' || label) && (label || (importing ? 'Importing...' : 'Import'))}
      </motion.button>
      <input
        ref={fileInputRef}
        type="file"
        accept=".json"
        style={{ display: 'none' }}
        onChange={handleFile}
      />
    </div>
  );
}

/* ── Import from Shared Directory Button ── */
interface SharedImportButtonProps {
  onClick: () => void;
  size?: 'sm' | 'md';
}

export function SharedImportButton({ onClick, size = 'sm' }: SharedImportButtonProps) {
  return (
    <motion.button
      {...buttonHover}
      className={`btn btn-secondary btn-${size}`}
      onClick={onClick}
      style={{
        display: 'flex', alignItems: 'center', gap: 6,
        ...(size === 'md' ? { padding: '8px 15px', fontSize: 12, borderRadius: 8 } : {}),
      }}
      title="Import from shared directory"
    >
      <FolderOpen size={12} />
      {size === 'md' && 'Shared'}
    </motion.button>
  );
}
