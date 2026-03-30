'use client';

import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  X, FolderOpen, FileText, ChevronDown, ChevronUp,
  Clock, Loader2, Code2, CheckCircle2, XCircle, Filter,
} from 'lucide-react';

const SECTION_COLOR = '#06b6d4';

interface RunFolder {
  runId: string;
  workflowSlug: string;
  workflowName: string;
  artifactDir: string;
  fileCount: number;
  totalSize: number;
  status: string;
  totalTokens: number;
  durationSec: number;
  timestamp: string;
  files: Array<{
    fileName: string;
    filePath: string;
    sizeBytes: number;
    mimeType: string;
    category?: string;
  }>;
}

interface WorkflowOutputDialogProps {
  workflowName: string;
  open: boolean;
  onClose: () => void;
}

function formatSize(bytes: number): string {
  if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(1)}MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)}KB`;
  return `${bytes}B`;
}

const STATUS_ICON: Record<string, { icon: typeof CheckCircle2; color: string }> = {
  completed: { icon: CheckCircle2, color: '#10b981' },
  failed:    { icon: XCircle, color: '#ef4444' },
  running:   { icon: Loader2, color: '#3b82f6' },
};

export default function WorkflowOutputDialog({ workflowName, open, onClose }: WorkflowOutputDialogProps) {
  const [runs, setRuns] = useState<RunFolder[]>([]);
  const [loading, setLoading] = useState(false);
  const [expandedRun, setExpandedRun] = useState<string | null>(null);
  const [fileContent, setFileContent] = useState<Record<string, string>>({});
  const [loadingFile, setLoadingFile] = useState<string | null>(null);
  const [expandedFile, setExpandedFile] = useState<string | null>(null);
  const [filterRunId, setFilterRunId] = useState('');

  useEffect(() => {
    if (!open || !workflowName) return;
    setLoading(true);
    fetch(`/api/workflows/outputs?workflowName=${encodeURIComponent(workflowName)}`)
      .then(r => r.json())
      .then(data => setRuns(data.runs ?? []))
      .catch(() => setRuns([]))
      .finally(() => setLoading(false));
    return () => { setRuns([]); setExpandedRun(null); setFileContent({}); setExpandedFile(null); };
  }, [open, workflowName]);

  const handleViewFile = async (run: RunFolder, fileName: string) => {
    const key = `${run.runId}/${fileName}`;
    if (expandedFile === key) { setExpandedFile(null); return; }
    setExpandedFile(key);
    if (fileContent[key]) return;
    setLoadingFile(key);
    try {
      const res = await fetch(
        `/api/workflows/outputs/file?workflowName=${encodeURIComponent(workflowName)}&runId=${encodeURIComponent(run.runId)}&filename=${encodeURIComponent(fileName)}`,
      );
      const data = await res.json();
      if (data.content) setFileContent(prev => ({ ...prev, [key]: data.content }));
    } catch { /* ignore */ }
    setLoadingFile(null);
  };

  // Version labels (newest = highest version)
  const versionedRuns = runs.map((r, i) => ({ ...r, version: runs.length - i }));

  const filteredRuns = filterRunId
    ? versionedRuns.filter(r => r.runId.toLowerCase().includes(filterRunId.toLowerCase()) || `v${r.version}` === filterRunId.toLowerCase())
    : versionedRuns.slice(0, 10);

  if (!open) return null;

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
          style={{
            position: 'fixed', inset: 0, zIndex: 1000,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            background: 'rgba(0,0,0,0.5)', backdropFilter: 'blur(4px)',
          }}
          onClick={onClose}
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 20 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 20 }}
            transition={{ type: 'spring', stiffness: 400, damping: 30 }}
            onClick={e => e.stopPropagation()}
            style={{
              background: 'var(--bg-surface)', border: '1px solid var(--border)',
              borderRadius: 16, width: '100%', maxWidth: 720,
              maxHeight: '85vh', display: 'flex', flexDirection: 'column',
              overflow: 'hidden',
            }}
          >
            {/* Header */}
            <div style={{
              display: 'flex', alignItems: 'center', justifyContent: 'space-between',
              padding: '18px 24px', borderBottom: '1px solid var(--border)',
            }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <FolderOpen size={18} color={SECTION_COLOR} />
                <div>
                  <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>
                    Workflow Outputs
                  </div>
                  <div style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 2 }}>
                    {workflowName} — {runs.length} run{runs.length !== 1 ? 's' : ''}
                  </div>
                </div>
              </div>
              <button onClick={onClose} style={{
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                width: 30, height: 30, borderRadius: 8, border: '1px solid var(--border)',
                background: 'transparent', cursor: 'pointer', color: 'var(--text-3)',
              }}>
                <X size={14} />
              </button>
            </div>

            {/* Version selector + filter */}
            {runs.length > 0 && (
              <div style={{ padding: '8px 24px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', gap: 8 }}>
                <select
                  value={filterRunId}
                  onChange={e => setFilterRunId(e.target.value)}
                  style={{
                    padding: '4px 8px', borderRadius: 6, fontSize: 11, fontWeight: 500,
                    border: '1px solid var(--border)', background: 'var(--bg-elevated)',
                    color: 'var(--text-1)', cursor: 'pointer', minWidth: 180,
                  }}
                >
                  <option value="">Latest 10 runs</option>
                  {versionedRuns.map(r => (
                    <option key={r.runId} value={r.runId}>v{r.version} — {r.runId.slice(0, 30)}</option>
                  ))}
                </select>
                <span style={{ fontSize: 10, color: 'var(--text-4)' }}>
                  {filteredRuns.length} of {runs.length} runs
                </span>
              </div>
            )}

            {/* Content */}
            <div style={{ flex: 1, overflow: 'auto', padding: '16px 24px' }}>
              {loading && (
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 40 }}>
                  <Loader2 size={20} className="spin" color={SECTION_COLOR} />
                </div>
              )}

              {!loading && runs.length === 0 && (
                <div style={{ textAlign: 'center', padding: '40px 0', color: 'var(--text-4)', fontSize: 13 }}>
                  No outputs yet. Run the workflow to generate outputs.
                </div>
              )}

              {!loading && filteredRuns.map(run => {
                const isExpanded = expandedRun === run.runId;
                const si = STATUS_ICON[run.status] ?? STATUS_ICON.completed;
                const StatusIcon = si.icon;
                return (
                  <div key={run.runId} style={{ marginBottom: 8 }}>
                    <button
                      onClick={() => setExpandedRun(isExpanded ? null : run.runId)}
                      style={{
                        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                        width: '100%', padding: '12px 14px', borderRadius: 10, cursor: 'pointer',
                        border: isExpanded ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border)',
                        background: isExpanded ? `${SECTION_COLOR}08` : 'var(--bg-surface)',
                        transition: 'all 0.15s',
                      }}
                    >
                      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                        <StatusIcon size={13} color={si.color} className={run.status === 'running' ? 'spin' : ''} />
                        <span style={{ fontSize: 10, fontWeight: 700, color: SECTION_COLOR, padding: '1px 5px', borderRadius: 3, background: `${SECTION_COLOR}15` }}>v{run.version}</span>
                        <span style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-1)', fontFamily: 'monospace' }}>
                          {run.runId.slice(0, 28)}
                        </span>
                      </div>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                        {run.totalTokens > 0 && (
                          <span style={{ fontSize: 10, color: 'var(--text-4)' }}>{run.totalTokens.toLocaleString()} tok</span>
                        )}
                        <span style={{ fontSize: 11, color: 'var(--text-4)' }}>
                          {run.fileCount} files · {formatSize(run.totalSize)}
                        </span>
                        {isExpanded ? <ChevronUp size={13} color="var(--text-4)" /> : <ChevronDown size={13} color="var(--text-4)" />}
                      </div>
                    </button>

                    <AnimatePresence>
                      {isExpanded && (
                        <motion.div
                          initial={{ opacity: 0, height: 0 }}
                          animate={{ opacity: 1, height: 'auto' }}
                          exit={{ opacity: 0, height: 0 }}
                          transition={{ duration: 0.2 }}
                          style={{ overflow: 'hidden' }}
                        >
                          <div style={{ padding: '8px 0', display: 'flex', flexDirection: 'column', gap: 4 }}>
                            {run.files.map(f => {
                              const key = `${run.runId}/${f.fileName}`;
                              const content = fileContent[key];
                              const isFileExpanded = expandedFile === key;
                              const isText = /\.(md|json|txt|py|sh|js|ts|yaml|yml)$/.test(f.fileName);
                              return (
                                <div key={f.fileName}>
                                  <button
                                    onClick={() => isText && handleViewFile(run, f.fileName)}
                                    style={{
                                      display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                                      width: '100%', padding: '8px 12px', borderRadius: 7,
                                      border: isFileExpanded ? `1px solid ${SECTION_COLOR}40` : '1px solid var(--border)',
                                      background: isFileExpanded ? `${SECTION_COLOR}06` : 'var(--bg-elevated)',
                                      cursor: isText ? 'pointer' : 'default',
                                      transition: 'all 0.15s',
                                    }}
                                  >
                                    <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                                      {f.category === 'script' ? <Code2 size={11} color="#3b82f6" /> : <FileText size={11} color="var(--text-4)" />}
                                      <span style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)' }}>{f.fileName}</span>
                                      {loadingFile === key && <Loader2 size={10} className="spin" color="var(--text-4)" />}
                                    </div>
                                    <span style={{ fontSize: 10, color: 'var(--text-4)' }}>{formatSize(f.sizeBytes)}</span>
                                  </button>
                                  {isFileExpanded && content && (
                                    <div style={{
                                      margin: '4px 0 4px 20px', padding: '10px 14px',
                                      borderRadius: 8, background: 'var(--bg-elevated)',
                                      border: '1px solid var(--border)',
                                      maxHeight: 300, overflow: 'auto',
                                    }}>
                                      <pre style={{
                                        margin: 0, fontSize: 11, lineHeight: 1.5,
                                        color: 'var(--text-2)', fontFamily: 'monospace',
                                        whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                                      }}>{content}</pre>
                                    </div>
                                  )}
                                </div>
                              );
                            })}
                          </div>
                        </motion.div>
                      )}
                    </AnimatePresence>
                  </div>
                );
              })}
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
