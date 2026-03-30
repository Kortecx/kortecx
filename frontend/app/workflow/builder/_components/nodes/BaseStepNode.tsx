'use client';

import { memo } from 'react';
import { Handle, Position, type NodeProps } from '@xyflow/react';

export type StepNodeType = 'start' | 'agent' | 'mcp-server' | 'executable' | 'action' | 'integration' | 'cloud-model' | 'master-agent';

export interface StepNodeData {
  label: string;
  stepType: StepNodeType;
  icon: string;
  color: string;
  status?: 'idle' | 'running' | 'completed' | 'failed';
  envLabel?: string;
  config?: Record<string, unknown>;
  onConfigure?: (id: string) => void;
  onDelete?: (id: string) => void;
}

const STATUS_DOT: Record<string, string> = {
  idle: '#6b7280',
  running: '#3b82f6',
  completed: '#10b981',
  failed: '#ef4444',
};

const TYPE_META: Record<StepNodeType, { label: string; abbr: string }> = {
  'start':        { label: 'Start',        abbr: 'START' },
  'agent':        { label: 'Agent',        abbr: 'AG' },
  'mcp-server':   { label: 'MCP Server',   abbr: 'MCP' },
  'executable':   { label: 'Executable',   abbr: 'EXE' },
  'action':       { label: 'Action',       abbr: 'ACT' },
  'integration':  { label: 'Integration',  abbr: 'INT' },
  'cloud-model':  { label: 'Cloud Model',  abbr: 'CLM' },
  'master-agent': { label: 'Master Agent', abbr: 'MA' },
};

function BaseStepNode({ id, data }: NodeProps & { data: StepNodeData }) {
  const meta = TYPE_META[data.stepType] ?? TYPE_META.agent;
  const statusColor = STATUS_DOT[data.status ?? 'idle'];
  const isStart = data.stepType === 'start';

  return (
    <div style={{
      background: 'var(--bg-surface)',
      border: isStart ? `2px solid ${data.color}` : '1px solid var(--border)',
      borderRadius: 10,
      padding: isStart ? '10px 16px' : '10px 14px',
      minWidth: isStart ? 80 : 150,
      maxWidth: 190,
      boxShadow: '0 2px 8px rgba(0,0,0,0.06)',
      position: 'relative',
    }}>
      {/* Input handle (not on start) */}
      {!isStart && (
        <Handle
          type="target"
          position={Position.Left}
          style={{ width: 8, height: 8, background: data.color, border: '2px solid var(--bg-surface)' }}
        />
      )}

      {/* Output handle */}
      <Handle
        type="source"
        position={Position.Right}
        style={{ width: 8, height: 8, background: data.color, border: '2px solid var(--bg-surface)' }}
      />

      {/* Top accent */}
      <div style={{
        position: 'absolute', top: 0, left: 0, right: 0, height: 2,
        background: data.color, borderRadius: '10px 10px 0 0',
      }} />

      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginTop: 1 }}>
        <span style={{ fontSize: isStart ? 14 : 15 }}>{data.icon}</span>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{
            fontSize: 11, fontWeight: 700, color: 'var(--text-1)',
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>
            {data.label}
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 4, marginTop: 1 }}>
            <span style={{
              fontSize: 7, padding: '1px 4px', borderRadius: 3, fontWeight: 700,
              background: `${data.color}18`, color: data.color,
              textTransform: 'uppercase', letterSpacing: '0.04em',
            }}>
              {meta.abbr}
            </span>
            {!isStart && (
              <div style={{ width: 5, height: 5, borderRadius: '50%', background: statusColor }} />
            )}
          </div>
        </div>
      </div>

      {/* Env label for running Docker steps */}
      {data.status === 'running' && data.envLabel && (
        <div style={{
          marginTop: 4, fontSize: 9, fontWeight: 600, color: '#3b82f6',
          fontFamily: 'monospace', letterSpacing: '0.02em',
        }}>
          Running in {data.envLabel}
        </div>
      )}

      {/* Action buttons (not on start) */}
      {!isStart && (
        <div style={{ display: 'flex', gap: 3, marginTop: 6 }}>
          {data.onConfigure && (
            <button
              onClick={(e) => { e.stopPropagation(); data.onConfigure!(id); }}
              style={{
                flex: 1, padding: '3px 0', borderRadius: 4, fontSize: 9, fontWeight: 600,
                border: `1px solid ${data.color}40`, background: `${data.color}08`,
                color: data.color, cursor: 'pointer',
              }}
            >
              Configure
            </button>
          )}
          {data.onDelete && (
            <button
              onClick={(e) => { e.stopPropagation(); data.onDelete!(id); }}
              style={{
                padding: '3px 6px', borderRadius: 4, fontSize: 9,
                border: '1px solid #ef444430', background: '#ef444408',
                color: '#ef4444', cursor: 'pointer',
              }}
            >
              ×
            </button>
          )}
        </div>
      )}
    </div>
  );
}

export default memo(BaseStepNode);
