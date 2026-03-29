'use client';

import { useState, useEffect, useCallback } from 'react';
import { X, FolderOpen, Download, Loader2, Filter, Check, AlertCircle } from 'lucide-react';
import { motion, AnimatePresence } from 'framer-motion';
import { buttonHover } from '@/lib/motion';
import type { ExportEntityType } from '@/lib/config-export';

interface SharedConfig {
  filename: string;
  entityType: ExportEntityType;
  name: string;
  exportedAt: string | null;
  version: string | null;
  sizeBytes: number;
}

interface SharedConfigImportDialogProps {
  open: boolean;
  onClose: () => void;
  onImported: (result: { id: string; name: string; entityType: string }) => void;
  filterType?: ExportEntityType;
}

const TYPE_LABELS: Record<string, { label: string; color: string }> = {
  expert: { label: 'Expert', color: '#7C3AED' },
  workflow: { label: 'Workflow', color: '#2563EB' },
  dataset: { label: 'Dataset', color: '#059669' },
  mcp_server: { label: 'MCP Server', color: '#D97706' },
  connection: { label: 'Connection', color: '#EC4899' },
  alert_rule: { label: 'Alert Rule', color: '#DC2626' },
};

export default function SharedConfigImportDialog({
  open, onClose, onImported, filterType,
}: SharedConfigImportDialogProps) {
  const [configs, setConfigs] = useState<SharedConfig[]>([]);
  const [loading, setLoading] = useState(false);
  const [importing, setImporting] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [typeFilter, setTypeFilter] = useState<ExportEntityType | 'all'>(filterType || 'all');
  const [dirExists, setDirExists] = useState(true);
  const [directory, setDirectory] = useState('');

  const fetchConfigs = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch('/api/shared-configs');
      const data = await res.json();
      setConfigs(data.configs || []);
      setDirExists(data.exists !== false);
      setDirectory(data.directory || '');
    } catch {
      setError('Failed to load shared configs');
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    if (open) fetchConfigs();
  }, [open, fetchConfigs]);

  const handleImport = async (filename: string) => {
    setImporting(filename);
    setError(null);
    setSuccess(null);

    try {
      const res = await fetch('/api/shared-configs/import', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ filename }),
      });

      if (!res.ok) {
        const err = await res.json().catch(() => ({ error: 'Import failed' }));
        throw new Error(err.error || 'Import failed');
      }

      const result = await res.json();
      setSuccess(`Imported "${result.name}" successfully`);
      onImported(result);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Import failed');
    }
    setImporting(null);
  };

  if (!open) return null;

  const filtered = typeFilter === 'all'
    ? configs
    : configs.filter(c => c.entityType === typeFilter);

  const typeOptions = ['all', ...new Set(configs.map(c => c.entityType))] as const;

  return (
    <AnimatePresence>
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        style={{
          position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.6)',
          backdropFilter: 'blur(4px)', zIndex: 200,
          display: 'flex', alignItems: 'flex-start', justifyContent: 'center', paddingTop: 80,
        }}
        onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
      >
        <motion.div
          initial={{ opacity: 0, scale: 0.96, y: 12 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          exit={{ opacity: 0, scale: 0.96, y: 12 }}
          onClick={(e) => e.stopPropagation()}
          style={{
            width: 520, maxWidth: '92vw', background: 'var(--bg-surface)',
            border: '1px solid var(--border)', borderRadius: 12, overflow: 'hidden',
            boxShadow: '0 24px 64px rgba(0,0,0,0.3)',
          }}
        >
          {/* Header */}
          <div style={{ padding: '18px 22px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', gap: 10 }}>
            <div style={{
              width: 36, height: 36, borderRadius: 8,
              background: 'rgba(37,99,235,0.1)', border: '1px solid rgba(37,99,235,0.25)',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}>
              <FolderOpen size={18} color="#2563EB" />
            </div>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>
                Import from Shared Directory
              </div>
              <div style={{ fontSize: 11, color: 'var(--text-4)', fontFamily: 'var(--font-mono, monospace)' }}>
                {directory || 'shared_configs/'}
              </div>
            </div>
            <button onClick={onClose}
              style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-3)', display: 'flex', padding: 4 }}>
              <X size={16} />
            </button>
          </div>

          {/* Filter bar */}
          <div style={{ padding: '12px 22px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', gap: 6 }}>
            <Filter size={11} color="var(--text-4)" />
            {typeOptions.map(t => (
              <button key={t} onClick={() => setTypeFilter(t as ExportEntityType | 'all')} style={{
                padding: '3px 10px', borderRadius: 5, fontSize: 10, fontWeight: typeFilter === t ? 600 : 400,
                border: `1px solid ${typeFilter === t ? '#2563EB' : 'var(--border)'}`,
                background: typeFilter === t ? 'rgba(37,99,235,0.08)' : 'transparent',
                color: typeFilter === t ? '#2563EB' : 'var(--text-3)',
                cursor: 'pointer', textTransform: 'capitalize',
              }}>
                {t === 'all' ? 'All' : TYPE_LABELS[t]?.label || t}
              </button>
            ))}
          </div>

          {/* Content */}
          <div style={{ padding: '16px 22px', maxHeight: '50vh', overflow: 'auto' }}>
            {/* Status messages */}
            {error && (
              <div style={{
                padding: '8px 12px', borderRadius: 6, fontSize: 11, marginBottom: 12,
                background: 'rgba(220,38,38,0.08)', border: '1px solid rgba(220,38,38,0.2)',
                color: '#DC2626', display: 'flex', alignItems: 'center', gap: 6,
              }}>
                <AlertCircle size={12} /> {error}
              </div>
            )}
            {success && (
              <div style={{
                padding: '8px 12px', borderRadius: 6, fontSize: 11, marginBottom: 12,
                background: 'rgba(5,150,105,0.08)', border: '1px solid rgba(5,150,105,0.2)',
                color: '#059669', display: 'flex', alignItems: 'center', gap: 6,
              }}>
                <Check size={12} /> {success}
              </div>
            )}

            {loading ? (
              <div style={{ padding: 40, textAlign: 'center', color: 'var(--text-3)', fontSize: 12 }}>
                <Loader2 size={16} style={{ margin: '0 auto 8px', animation: 'spin 1s linear infinite', display: 'block' }} />
                Loading configs...
              </div>
            ) : !dirExists ? (
              <div style={{ padding: 40, textAlign: 'center', color: 'var(--text-3)', fontSize: 12 }}>
                <FolderOpen size={24} color="var(--text-4)" style={{ margin: '0 auto 8px' }} />
                <div style={{ fontWeight: 600, color: 'var(--text-2)', marginBottom: 4 }}>Shared directory not found</div>
                <div>Create a <code style={{ background: 'var(--bg)', padding: '1px 4px', borderRadius: 3 }}>shared_configs/</code> directory or set <code style={{ background: 'var(--bg)', padding: '1px 4px', borderRadius: 3 }}>KORTECX_SHARED_CONFIG_DIR</code></div>
              </div>
            ) : filtered.length === 0 ? (
              <div style={{ padding: 40, textAlign: 'center', color: 'var(--text-3)', fontSize: 12 }}>
                <FolderOpen size={24} color="var(--text-4)" style={{ margin: '0 auto 8px' }} />
                <div style={{ fontWeight: 600, color: 'var(--text-2)', marginBottom: 4 }}>No configs found</div>
                <div>Place exported <code style={{ background: 'var(--bg)', padding: '1px 4px', borderRadius: 3 }}>.json</code> files in the shared directory</div>
              </div>
            ) : (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                {filtered.map((config) => {
                  const typeInfo = TYPE_LABELS[config.entityType] || { label: config.entityType, color: '#6b7280' };
                  return (
                    <div key={config.filename} className="card" style={{
                      padding: '12px 14px', display: 'flex', alignItems: 'center', gap: 10,
                    }}>
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                          {config.name}
                        </div>
                        <div style={{ display: 'flex', gap: 6, alignItems: 'center', marginTop: 3 }}>
                          <span style={{
                            fontSize: 9, fontWeight: 700, padding: '1px 6px', borderRadius: 3,
                            background: `${typeInfo.color}12`, color: typeInfo.color, textTransform: 'uppercase',
                          }}>
                            {typeInfo.label}
                          </span>
                          <span style={{ fontSize: 10, color: 'var(--text-4)' }}>
                            {config.sizeBytes > 1024 ? `${(config.sizeBytes / 1024).toFixed(1)} KB` : `${config.sizeBytes} B`}
                          </span>
                          {config.exportedAt && (
                            <span style={{ fontSize: 10, color: 'var(--text-4)' }}>
                              {new Date(config.exportedAt).toLocaleDateString()}
                            </span>
                          )}
                        </div>
                      </div>
                      <motion.button
                        {...buttonHover}
                        className="btn btn-primary btn-sm"
                        onClick={() => handleImport(config.filename)}
                        disabled={importing === config.filename}
                        style={{ display: 'flex', alignItems: 'center', gap: 4, flexShrink: 0 }}
                      >
                        {importing === config.filename
                          ? <Loader2 size={11} style={{ animation: 'spin 1s linear infinite' }} />
                          : <Download size={11} />}
                        Import
                      </motion.button>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          {/* Footer */}
          <div style={{ padding: '14px 22px', borderTop: '1px solid var(--border)', display: 'flex', justifyContent: 'flex-end' }}>
            <button className="btn btn-secondary btn-sm" onClick={onClose}>Close</button>
          </div>
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}
