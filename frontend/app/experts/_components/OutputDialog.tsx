'use client';

import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  X, FolderOpen, FileText, ChevronDown, ChevronUp,
  Clock, Loader2, Code2,
} from 'lucide-react';

const SECTION_COLOR = '#D97706';

interface RunFolder {
  runTs: string;
  expertSlug: string;
  expertName: string;
  artifactDir: string;
  fileCount: number;
  totalSize: number;
  files: Array<{
    fileName: string;
    filePath: string;
    sizeBytes: number;
    mimeType: string;
    fileType: string;
    category?: string;
  }>;
}

interface OutputDialogProps {
  expertId: string;
  expertName: string;
  open: boolean;
  onClose: () => void;
}

function formatTs(ts: string): string {
  // 20260330_141523 → 2026-03-30 14:15:23
  if (ts.length < 15) return ts;
  return `${ts.slice(0, 4)}-${ts.slice(4, 6)}-${ts.slice(6, 8)} ${ts.slice(9, 11)}:${ts.slice(11, 13)}:${ts.slice(13, 15)}`;
}

function formatSize(bytes: number): string {
  if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(1)}MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)}KB`;
  return `${bytes}B`;
}

export default function OutputDialog({ expertId, expertName, open, onClose }: OutputDialogProps) {
  const [runs, setRuns] = useState<RunFolder[]>([]);
  const [loading, setLoading] = useState(false);
  const [expandedRun, setExpandedRun] = useState<string | null>(null);
  const [fileContent, setFileContent] = useState<Record<string, string>>({});
  const [loadingFile, setLoadingFile] = useState<string | null>(null);
  const [expandedFile, setExpandedFile] = useState<string | null>(null);

  useEffect(() => {
    if (!open || !expertId) return;
    setLoading(true);
    fetch(`/api/experts/outputs?expertId=${expertId}`)
      .then(r => r.json())
      .then(data => setRuns(data.runs ?? []))
      .catch(() => setRuns([]))
      .finally(() => setLoading(false));
    return () => { setRuns([]); setExpandedRun(null); setFileContent({}); setExpandedFile(null); };
  }, [open, expertId]);

  const handleViewFile = async (run: RunFolder, fileName: string) => {
    const key = `${run.runTs}/${fileName}`;
    // Toggle collapse if same file clicked again
    if (expandedFile === key) { setExpandedFile(null); return; }
    setExpandedFile(key);
    if (fileContent[key]) return; // already loaded
    setLoadingFile(key);
    try {
      const engineUrl = '/api/experts/outputs/file';
      const res = await fetch(
        `${engineUrl}?expertId=${expertId}&runTs=${run.runTs}&filename=${encodeURIComponent(fileName)}`,
      );
      const data = await res.json();
      if (data.content) {
        setFileContent(prev => ({ ...prev, [key]: data.content }));
      }
    } catch { /* ignore */ }
    setLoadingFile(null);
  };

  if (!open) return null;

  const categoryIcon = (cat?: string) => {
    if (cat === 'script') return <Code2 size={11} color="#3b82f6" />;
    return <FileText size={11} color="var(--text-4)" />;
  };

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
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
              borderRadius: 16, width: '100%', maxWidth: 680,
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
                    Agent Outputs
                  </div>
                  <div style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 2 }}>
                    {expertName} — outputs/agents/{expertName.toLowerCase().replace(/[^a-z0-9]+/g, '-')}/
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

            {/* Content */}
            <div style={{ flex: 1, overflow: 'auto', padding: '16px 24px' }}>
              {loading && (
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 40 }}>
                  <Loader2 size={20} className="spin" color={SECTION_COLOR} />
                </div>
              )}

              {!loading && runs.length === 0 && (
                <div style={{
                  textAlign: 'center', padding: '40px 0',
                  color: 'var(--text-4)', fontSize: 13,
                }}>
                  No outputs yet. Run the agent to generate outputs.
                </div>
              )}

              {!loading && runs.map(run => {
                const isExpanded = expandedRun === run.runTs;
                return (
                  <div key={run.runTs} style={{ marginBottom: 8 }}>
                    {/* Run header */}
                    <button
                      onClick={() => setExpandedRun(isExpanded ? null : run.runTs)}
                      style={{
                        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                        width: '100%', padding: '12px 14px', borderRadius: 10, cursor: 'pointer',
                        border: isExpanded ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border)',
                        background: isExpanded ? `${SECTION_COLOR}08` : 'var(--bg-surface)',
                        transition: 'all 0.15s',
                      }}
                    >
                      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                        <Clock size={13} color={isExpanded ? SECTION_COLOR : 'var(--text-4)'} />
                        <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', fontFamily: 'monospace' }}>
                          {formatTs(run.runTs)}
                        </span>
                      </div>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                        <span style={{ fontSize: 11, color: 'var(--text-4)' }}>
                          {run.fileCount} files · {formatSize(run.totalSize)}
                        </span>
                        {isExpanded ? <ChevronUp size={13} color="var(--text-4)" /> : <ChevronDown size={13} color="var(--text-4)" />}
                      </div>
                    </button>

                    {/* Expanded file list */}
                    <AnimatePresence>
                      {isExpanded && (
                        <motion.div
                          initial={{ opacity: 0, height: 0 }}
                          animate={{ opacity: 1, height: 'auto' }}
                          exit={{ opacity: 0, height: 0 }}
                          transition={{ duration: 0.2 }}
                          style={{ overflow: 'hidden' }}
                        >
                          <div style={{
                            padding: '8px 0 0 0',
                            display: 'flex', flexDirection: 'column', gap: 4,
                          }}>
                            {run.files.map(f => {
                              const key = `${run.runTs}/${f.fileName}`;
                              const content = fileContent[key];
                              const isText = f.mimeType?.startsWith('text/') ||
                                f.fileName.endsWith('.md') || f.fileName.endsWith('.json') ||
                                f.fileName.endsWith('.txt') || f.fileName.endsWith('.py') ||
                                f.fileName.endsWith('.sh') || f.fileName.endsWith('.js') ||
                                f.fileName.endsWith('.ts');
                              return (
                                <div key={f.fileName}>
                                  <button
                                    onClick={() => isText && handleViewFile(run, f.fileName)}
                                    style={{
                                      display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                                      width: '100%', padding: '8px 12px', borderRadius: 7,
                                      border: expandedFile === key ? `1px solid ${SECTION_COLOR}40` : '1px solid var(--border)',
                                      background: expandedFile === key ? `${SECTION_COLOR}06` : 'var(--bg-elevated)',
                                      cursor: isText ? 'pointer' : 'default',
                                      transition: 'all 0.15s',
                                    }}
                                  >
                                    <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                                      {categoryIcon(f.category)}
                                      <span style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)' }}>
                                        {f.fileName}
                                      </span>
                                      {loadingFile === key && <Loader2 size={10} className="spin" color="var(--text-4)" />}
                                    </div>
                                    <span style={{ fontSize: 10, color: 'var(--text-4)' }}>
                                      {formatSize(f.sizeBytes)}
                                    </span>
                                  </button>
                                  {/* Inline content preview */}
                                  {expandedFile === key && content && (
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
                                      }}>
                                        {content}
                                      </pre>
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
