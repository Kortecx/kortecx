'use client';

import { motion } from 'framer-motion';
import {
  Zap, Clock, Loader2, CheckCircle2, FileText,
  Server, AlertCircle,
} from 'lucide-react';

const SECTION_COLOR = '#D97706';

const fadeUp = {
  hidden: { opacity: 0, y: 14 },
  show:   { opacity: 1, y: 0, transition: { duration: 0.32, ease: [0.25, 0.46, 0.45, 0.94] as const } },
};

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

function elapsed(iso: string) {
  const d = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (d < 60) return `${d}s ago`;
  if (d < 3600) return `${Math.floor(d / 60)}m ago`;
  return `${Math.floor(d / 3600)}h ago`;
}

export interface ExpertRun {
  id: string;
  expertId: string;
  expertName: string;
  status: string;
  model: string;
  engine: string;
  temperature: string;
  maxTokens: number;
  tokensUsed: number;
  durationMs: number;
  artifactCount: number;
  errorMessage: string | null;
  metadata: Record<string, unknown> | null;
  startedAt: string | null;
  completedAt: string | null;
  createdAt: string;
}

interface ExpertRunCardProps {
  run: ExpertRun;
  onClick: () => void;
}

export default function ExpertRunCard({ run, onClick }: ExpertRunCardProps) {
  const statusCfg = STATUS_CONFIG[run.status] ?? STATUS_CONFIG.running;
  const StatusIcon = statusCfg.icon;
  const isRunning = run.status === 'running';

  return (
    <motion.div
      variants={fadeUp}
      whileHover={{ y: -2, boxShadow: '0 8px 24px rgba(13,13,13,0.08)' }}
      transition={{ type: 'spring', stiffness: 380, damping: 28 }}
      onClick={onClick}
      style={{
        background: 'var(--bg-surface)',
        border: '1px solid var(--border)',
        borderRadius: 12, padding: 18,
        display: 'flex', flexDirection: 'column', gap: 12,
        position: 'relative', overflow: 'hidden', cursor: 'pointer',
      }}
    >
      {/* Top accent */}
      <div style={{
        position: 'absolute', top: 0, left: 0, right: 0, height: 2,
        background: `linear-gradient(90deg, ${statusCfg.color}, ${statusCfg.color}50)`,
        borderRadius: '12px 12px 0 0',
      }} />

      {/* Header: name + status */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginTop: 2 }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1.2 }}>
            {run.expertName}
          </div>
          <div style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 3, fontFamily: 'monospace' }}>
            {run.id}
          </div>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexShrink: 0 }}>
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

      {/* Model + engine */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 8,
        paddingBottom: 10, borderBottom: '1px solid var(--border)',
      }}>
        <Server size={11} color="var(--text-4)" />
        <span style={{ fontSize: 12, color: 'var(--text-3)', fontWeight: 500 }}>
          {run.model}
        </span>
        <div style={{
          padding: '1px 7px', borderRadius: 99, fontSize: 10, fontWeight: 700,
          background: `${SECTION_COLOR}14`, color: SECTION_COLOR,
          border: `1px solid ${SECTION_COLOR}28`, marginLeft: 'auto',
        }}>
          {run.engine}
        </div>
      </div>

      {/* Stats */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 8 }}>
        <StatBox
          icon={Zap} label="Tokens" color="#3b82f6"
          value={run.tokensUsed > 0 ? fmt(run.tokensUsed) : '\u2014'}
        />
        <StatBox
          icon={Clock} label="Duration" color="#f59e0b"
          value={run.durationMs > 0 ? `${(run.durationMs / 1000).toFixed(1)}s` : isRunning ? 'Running...' : '\u2014'}
        />
        <StatBox
          icon={FileText} label="Artifacts" color="#10b981"
          value={String(run.artifactCount ?? 0)}
        />
      </div>

      {/* Error message */}
      {run.errorMessage && (
        <div style={{
          padding: '8px 10px', borderRadius: 6,
          background: 'rgba(239,68,68,0.06)', border: '1px solid rgba(239,68,68,0.15)',
          fontSize: 11, color: '#ef4444', lineHeight: 1.4,
          overflow: 'hidden', textOverflow: 'ellipsis',
          display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical',
        }}>
          {run.errorMessage}
        </div>
      )}

      {/* Timestamps */}
      <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 10, color: 'var(--text-4)' }}>
        <span>Started {run.startedAt ? elapsed(run.startedAt) : elapsed(run.createdAt)}</span>
        {run.completedAt && <span>Completed {elapsed(run.completedAt)}</span>}
      </div>
    </motion.div>
  );
}

function StatBox({ icon: Icon, label, value, color }: {
  icon: typeof Zap; label: string; value: string; color: string;
}) {
  return (
    <div style={{
      textAlign: 'center', padding: '7px 4px', borderRadius: 7,
      background: `${color}06`, border: `1px solid ${color}12`,
    }}>
      <div style={{ display: 'flex', justifyContent: 'center', marginBottom: 3 }}>
        <Icon size={10} color={color} strokeWidth={2.5} />
      </div>
      <div style={{ fontSize: 13, fontWeight: 800, color: 'var(--text-1)', lineHeight: 1 }}>
        {value}
      </div>
      <div style={{ fontSize: 9, color: 'var(--text-4)', marginTop: 2, fontWeight: 500 }}>
        {label}
      </div>
    </div>
  );
}
