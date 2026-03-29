'use client';

import { memo } from 'react';
import { Handle, Position, type NodeProps } from '@xyflow/react';

const STATUS_COLORS: Record<string, { bg: string; border: string; text: string }> = {
  pending:   { bg: '#f3f4f6', border: '#d1d5db', text: '#6b7280' },
  running:   { bg: '#dbeafe', border: '#3b82f6', text: '#1d4ed8' },
  completed: { bg: '#dcfce7', border: '#22c55e', text: '#15803d' },
  failed:    { bg: '#fef2f2', border: '#ef4444', text: '#b91c1c' },
};

const ROLE_EMOJI: Record<string, string> = {
  researcher: '🔬', analyst: '📊', writer: '✍️', coder: '💻',
  reviewer: '🔍', planner: '🗂', legal: '⚖️', financial: '💰',
  medical: '🩺', coordinator: '🔄', 'data-engineer': '🛠', creative: '🎨',
  translator: '🌐', custom: '⚙️',
};

interface RunNodeData {
  label: string;
  role?: string;
  status: string;
  tokensUsed?: number;
  durationMs?: number;
  agentId?: string;
  [key: string]: unknown;
}

function RunGraphNode({ data }: NodeProps & { data: RunNodeData }) {
  const status = STATUS_COLORS[data.status] ?? STATUS_COLORS.pending;
  const emoji = ROLE_EMOJI[(data.role as string) ?? 'custom'] ?? '⚙️';
  const isRunning = data.status === 'running';

  return (
    <div style={{
      padding: '10px 14px', borderRadius: 10, minWidth: 160,
      background: status.bg, border: `2px solid ${status.border}`,
      boxShadow: isRunning ? `0 0 12px ${status.border}40` : '0 1px 4px rgba(0,0,0,0.08)',
      transition: 'all 0.3s',
    }}>
      <Handle type="target" position={Position.Top} style={{ background: status.border, width: 8, height: 8 }} />

      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
        <span style={{ fontSize: 16 }}>{emoji}</span>
        <span style={{ fontSize: 12, fontWeight: 700, color: '#1a1a1a' }}>
          {data.label}
        </span>
      </div>

      <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 10 }}>
        <span style={{
          padding: '1px 6px', borderRadius: 4,
          background: `${status.border}20`, color: status.text,
          fontWeight: 600, textTransform: 'uppercase',
        }}>
          {isRunning && <span style={{ display: 'inline-block', width: 6, height: 6, borderRadius: '50%', background: status.border, marginRight: 4, animation: 'pulse 1.5s infinite' }} />}
          {data.status}
        </span>
        {data.tokensUsed !== undefined && data.tokensUsed > 0 && (
          <span style={{ color: '#6b7280' }}>{data.tokensUsed} tok</span>
        )}
        {data.durationMs !== undefined && data.durationMs > 0 && (
          <span style={{ color: '#6b7280' }}>{(data.durationMs / 1000).toFixed(1)}s</span>
        )}
      </div>

      <Handle type="source" position={Position.Bottom} style={{ background: status.border, width: 8, height: 8 }} />
    </div>
  );
}

export default memo(RunGraphNode);
