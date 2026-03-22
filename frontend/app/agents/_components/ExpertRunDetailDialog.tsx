'use client';

import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  X, Zap, Clock, FileText, Server, ExternalLink,
  Loader2, CheckCircle2, AlertCircle, ChevronDown, ChevronUp,
  FolderOpen,
} from 'lucide-react';
import { useRouter } from 'next/navigation';
import type { ExpertRun } from './ExpertRunCard';

interface ExpertRunDetailDialogProps {
  run: ExpertRun | null;
  open: boolean;
  onClose: () => void;
}

const STATUS_CONFIG: Record<string, { color: string; label: string; icon: typeof Loader2 }> = {
  queued:    { color: '#f59e0b', label: 'Queued',    icon: Clock        },
  running:   { color: '#3b82f6', label: 'Running',   icon: Loader2      },
  completed: { color: '#10b981', label: 'Completed', icon: CheckCircle2 },
  failed:    { color: '#ef4444', label: 'Failed',    icon: AlertCircle  },
};

function fmt(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export default function ExpertRunDetailDialog({ run, open, onClose }: ExpertRunDetailDialogProps) {
  const router = useRouter();
  const [showPrompts, setShowPrompts] = useState(false);
  const [showResponse, setShowResponse] = useState(false);
  const [fullRun, setFullRun] = useState<Record<string, unknown> | null>(null);

  // Fetch full run data (includes responseText, prompts)
  useEffect(() => {
    if (!open || !run?.id) { setFullRun(null); return; }
    const fetchRun = async () => {
      try {
        const res = await fetch(`/api/experts/run?id=${run.id}`);
        if (res.ok) {
          const data = await res.json();
          if (data.runs?.length > 0) setFullRun(data.runs[0]);
        }
      } catch { /* ignore */ }
    };
    fetchRun();
    // Re-fetch while running
    if (run.status === 'running') {
      const interval = setInterval(fetchRun, 3000);
      return () => clearInterval(interval);
    }
  }, [open, run?.id, run?.status]);

  if (!open || !run) return null;

  const statusCfg = STATUS_CONFIG[run.status] ?? STATUS_CONFIG.running;
  const StatusIcon = statusCfg.icon;
  const isRunning = run.status === 'running';
  const meta = (fullRun?.metadata ?? run.metadata ?? {}) as Record<string, unknown>;
  const artifactDir = meta.artifactDir as string ?? '';
  const responseText = (fullRun?.responseText ?? '') as string;
  const systemPrompt = (fullRun?.systemPrompt ?? '') as string;
  const userPrompt = (fullRun?.userPrompt ?? '') as string;

  const handleViewAssets = () => {
    onClose();
    router.push('/data');
  };

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          onClick={onClose}
          style={{
            position: 'fixed', inset: 0, zIndex: 1000,
            background: 'rgba(0,0,0,0.5)', backdropFilter: 'blur(4px)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.95, y: 20 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.95, y: 20 }}
            transition={{ type: 'spring', stiffness: 400, damping: 30 }}
            onClick={e => e.stopPropagation()}
            style={{
              zIndex: 1001,
              width: 'min(92vw, 700px)',
              maxHeight: '85vh',
              background: 'var(--bg-surface)',
              border: '1px solid var(--border)',
              borderRadius: 16,
              display: 'flex', flexDirection: 'column',
              overflow: 'hidden',
              boxShadow: '0 24px 80px rgba(0,0,0,0.15)',
            }}
          >
            {/* Header */}
            <div style={{
              display: 'flex', alignItems: 'center', justifyContent: 'space-between',
              padding: '18px 24px', borderBottom: '1px solid var(--border)',
            }}>
              <div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                  <h2 style={{ fontSize: 16, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1.2 }}>
                    {run.expertName}
                  </h2>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
                    {isRunning ? (
                      <StatusIcon size={14} className="spin" color={statusCfg.color} />
                    ) : (
                      <StatusIcon size={14} color={statusCfg.color} />
                    )}
                    <span style={{ fontSize: 12, fontWeight: 600, color: statusCfg.color }}>
                      {statusCfg.label}
                    </span>
                  </div>
                </div>
                <div style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 3, fontFamily: 'monospace' }}>
                  {run.id}
                </div>
              </div>
              <button
                onClick={onClose}
                style={{
                  width: 32, height: 32, borderRadius: 8, border: '1px solid var(--border-md)',
                  background: 'transparent', cursor: 'pointer',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}
              >
                <X size={16} color="var(--text-3)" />
              </button>
            </div>

            {/* Content */}
            <div style={{ flex: 1, overflow: 'auto', padding: 24, display: 'flex', flexDirection: 'column', gap: 18 }}>

              {/* Stats grid */}
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 10 }}>
                {[
                  { label: 'Tokens Used',  value: run.tokensUsed > 0 ? fmt(run.tokensUsed) : '\u2014', icon: Zap, color: '#3b82f6' },
                  { label: 'Duration',     value: run.durationMs > 0 ? `${(run.durationMs / 1000).toFixed(1)}s` : isRunning ? '...' : '\u2014', icon: Clock, color: '#f59e0b' },
                  { label: 'Artifacts',    value: String(run.artifactCount ?? 0), icon: FileText, color: '#10b981' },
                  { label: 'Model',        value: run.model || '\u2014', icon: Server, color: '#8b5cf6' },
                ].map(({ label, value, icon: Icon, color }) => (
                  <div key={label} style={{
                    padding: '12px 14px', borderRadius: 10,
                    background: `${color}06`, border: `1px solid ${color}12`,
                  }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 5, marginBottom: 6 }}>
                      <Icon size={11} color={color} strokeWidth={2.5} />
                      <span style={{ fontSize: 10, color: 'var(--text-4)', fontWeight: 600 }}>{label}</span>
                    </div>
                    <div style={{
                      fontSize: 16, fontWeight: 800, color: 'var(--text-1)', lineHeight: 1,
                      overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                    }}>
                      {value}
                    </div>
                  </div>
                ))}
              </div>

              {/* Engine + timestamps */}
              <div style={{
                display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 12,
                padding: '12px 16px', borderRadius: 10,
                background: 'var(--bg-elevated)', border: '1px solid var(--border)',
              }}>
                <div>
                  <div style={{ fontSize: 10, color: 'var(--text-4)', marginBottom: 3 }}>Engine</div>
                  <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-2)' }}>{run.engine}</div>
                </div>
                <div>
                  <div style={{ fontSize: 10, color: 'var(--text-4)', marginBottom: 3 }}>Started</div>
                  <div style={{ fontSize: 12, color: 'var(--text-2)' }}>
                    {run.startedAt ? new Date(run.startedAt).toLocaleString() : '\u2014'}
                  </div>
                </div>
                <div>
                  <div style={{ fontSize: 10, color: 'var(--text-4)', marginBottom: 3 }}>Completed</div>
                  <div style={{ fontSize: 12, color: 'var(--text-2)' }}>
                    {run.completedAt ? new Date(run.completedAt).toLocaleString() : isRunning ? 'In progress...' : '\u2014'}
                  </div>
                </div>
              </div>

              {/* Error */}
              {run.errorMessage && (
                <div style={{
                  padding: '12px 14px', borderRadius: 8,
                  background: 'rgba(239,68,68,0.06)', border: '1px solid rgba(239,68,68,0.15)',
                }}>
                  <div style={{ fontSize: 10, color: '#ef4444', fontWeight: 700, marginBottom: 4 }}>ERROR</div>
                  <div style={{ fontSize: 12, color: '#ef4444', lineHeight: 1.5 }}>{run.errorMessage}</div>
                </div>
              )}

              {/* File Location */}
              {artifactDir && (
                <div style={{
                  padding: '12px 14px', borderRadius: 8,
                  background: 'var(--bg-elevated)', border: '1px solid var(--border)',
                }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 6 }}>
                    <FolderOpen size={12} color="#D97706" />
                    <span style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-2)' }}>Artifact Location</span>
                  </div>
                  <div style={{
                    fontSize: 12, color: 'var(--text-3)', fontFamily: 'monospace',
                    padding: '6px 10px', borderRadius: 6, background: 'var(--bg-surface)',
                    border: '1px solid var(--border)',
                  }}>
                    {artifactDir}
                  </div>
                </div>
              )}

              {/* Prompts (collapsible) */}
              {(systemPrompt || userPrompt) && (
                <button
                  onClick={() => setShowPrompts(!showPrompts)}
                  style={{
                    display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                    padding: '10px 14px', borderRadius: 8, cursor: 'pointer',
                    border: '1px solid var(--border)', background: 'var(--bg-elevated)',
                    width: '100%', textAlign: 'left',
                  }}
                >
                  <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-2)' }}>Prompts</span>
                  {showPrompts ? <ChevronUp size={14} color="var(--text-3)" /> : <ChevronDown size={14} color="var(--text-3)" />}
                </button>
              )}
              {showPrompts && (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
                  {systemPrompt && (
                    <div>
                      <div style={{ fontSize: 10, color: 'var(--text-4)', fontWeight: 600, marginBottom: 4 }}>SYSTEM PROMPT</div>
                      <pre style={{
                        fontSize: 11, color: 'var(--text-2)', lineHeight: 1.5,
                        padding: '10px 12px', borderRadius: 8, background: 'var(--bg-elevated)',
                        border: '1px solid var(--border)', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                        maxHeight: 150, overflow: 'auto', margin: 0,
                      }}>
                        {systemPrompt}
                      </pre>
                    </div>
                  )}
                  {userPrompt && (
                    <div>
                      <div style={{ fontSize: 10, color: 'var(--text-4)', fontWeight: 600, marginBottom: 4 }}>USER PROMPT</div>
                      <pre style={{
                        fontSize: 11, color: 'var(--text-2)', lineHeight: 1.5,
                        padding: '10px 12px', borderRadius: 8, background: 'var(--bg-elevated)',
                        border: '1px solid var(--border)', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                        maxHeight: 150, overflow: 'auto', margin: 0,
                      }}>
                        {userPrompt}
                      </pre>
                    </div>
                  )}
                </div>
              )}

              {/* Response (collapsible) */}
              {responseText && (
                <>
                  <button
                    onClick={() => setShowResponse(!showResponse)}
                    style={{
                      display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                      padding: '10px 14px', borderRadius: 8, cursor: 'pointer',
                      border: '1px solid var(--border)', background: 'var(--bg-elevated)',
                      width: '100%', textAlign: 'left',
                    }}
                  >
                    <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-2)' }}>Response</span>
                    {showResponse ? <ChevronUp size={14} color="var(--text-3)" /> : <ChevronDown size={14} color="var(--text-3)" />}
                  </button>
                  {showResponse && (
                    <pre style={{
                      fontSize: 11, color: 'var(--text-2)', lineHeight: 1.6,
                      padding: '12px 14px', borderRadius: 8, background: 'var(--bg-elevated)',
                      border: '1px solid var(--border)', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                      maxHeight: 300, overflow: 'auto', margin: 0,
                    }}>
                      {responseText}
                    </pre>
                  )}
                </>
              )}
            </div>

            {/* Footer */}
            <div style={{
              display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: 10,
              padding: '14px 24px', borderTop: '1px solid var(--border)',
            }}>
              <button
                onClick={onClose}
                style={{
                  padding: '8px 18px', borderRadius: 8, fontSize: 13,
                  border: '1px solid var(--border-md)',
                  background: 'transparent', color: 'var(--text-2)',
                  cursor: 'pointer',
                }}
              >
                Close
              </button>
              <button
                onClick={handleViewAssets}
                style={{
                  display: 'flex', alignItems: 'center', gap: 6,
                  padding: '8px 20px', borderRadius: 8, fontSize: 13, fontWeight: 600,
                  border: 'none',
                  background: '#D97706', color: '#fff', cursor: 'pointer',
                }}
              >
                <ExternalLink size={14} />
                View in Data Synthesis
              </button>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
