'use client';

import { motion } from 'framer-motion';
import { Plus, X, Bot, Shield } from 'lucide-react';

interface MasterAgent {
  expertId: string;
  name: string;
  role: string;
  model?: string;
  description?: string;
}

interface MasterAgentPanelProps {
  masterAgent: MasterAgent | null;
  onAttach: () => void;
  onDetach: () => void;
}

const ACCENT = '#06b6d4';

export default function MasterAgentPanel({ masterAgent, onAttach, onDetach }: MasterAgentPanelProps) {
  return (
    <div style={{
      background: 'var(--bg-surface)', border: '1px solid var(--border)',
      borderRadius: 8, padding: '10px 14px',
      display: 'flex', alignItems: 'center', gap: 10,
      minHeight: 72,
    }}>
      <Shield size={14} color={ACCENT} style={{ flexShrink: 0 }} />

      {!masterAgent ? (
        <button
          onClick={onAttach}
          style={{
            display: 'flex', alignItems: 'center', gap: 5, flex: 1,
            padding: 0, border: 'none', background: 'transparent',
            color: ACCENT, fontSize: 11, fontWeight: 600, cursor: 'pointer',
          }}
        >
          <Plus size={12} /> Attach Master Agent
        </button>
      ) : (
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, flex: 1, minWidth: 0 }}>
          <Bot size={14} color={ACCENT} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{
              fontSize: 12, fontWeight: 700, color: 'var(--text-1)',
              overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
            }}>
              {masterAgent.name}
            </div>
            <div style={{ fontSize: 9, color: 'var(--text-4)' }}>
              {masterAgent.role}{masterAgent.model ? ` · ${masterAgent.model}` : ''}
            </div>
          </div>
          <button
            onClick={onDetach}
            style={{
              width: 20, height: 20, borderRadius: 5, flexShrink: 0,
              border: '1px solid #ef444430', background: '#ef444408',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              cursor: 'pointer', color: '#ef4444',
            }}
          >
            <X size={9} />
          </button>
        </div>
      )}
    </div>
  );
}

export type { MasterAgent };
