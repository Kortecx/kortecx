'use client';

import { Plus, X, Bot, Shield, Users } from 'lucide-react';

interface MasterAgent {
  expertId: string;
  name: string;
  role: string;
  model?: string;
  description?: string;
}

interface MasterAgentPanelProps {
  masterAgent: MasterAgent | null;
  connectedAgents: MasterAgent[];
  onAttach: () => void;
  onDetach: () => void;
  onAttachConnected: () => void;
  onDetachConnected: (index: number) => void;
}

const ACCENT = '#06b6d4';
const MAX_CONNECTED = 4;

export default function MasterAgentPanel({ masterAgent, connectedAgents, onAttach, onDetach, onAttachConnected, onDetachConnected }: MasterAgentPanelProps) {
  return (
    <div style={{
      background: 'var(--bg-surface)', border: '1px solid var(--border)',
      borderRadius: 8, padding: '10px 14px',
      display: 'flex', flexDirection: 'column', gap: 8,
      minHeight: 72,
    }}>
      {/* Master agent row */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <Shield size={16} color={ACCENT} style={{ flexShrink: 0 }} />

        {!masterAgent ? (
          <button onClick={onAttach} style={{
            display: 'flex', alignItems: 'center', gap: 6, flex: 1,
            padding: 0, border: 'none', background: 'transparent',
            color: ACCENT, fontSize: 13, fontWeight: 600, cursor: 'pointer',
          }}>
            <Plus size={14} /> Attach Master Agent
          </button>
        ) : (
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, flex: 1, minWidth: 0 }}>
            <Bot size={14} color={ACCENT} />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--text-1)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {masterAgent.name}
              </div>
              <div style={{ fontSize: 9, color: 'var(--text-4)' }}>
                Master · {masterAgent.role}{masterAgent.model ? ` · ${masterAgent.model}` : ''}
              </div>
            </div>
            <button onClick={onDetach} style={{
              width: 20, height: 20, borderRadius: 5, flexShrink: 0,
              border: '1px solid #ef444430', background: '#ef444408',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              cursor: 'pointer', color: '#ef4444',
            }}>
              <X size={9} />
            </button>
          </div>
        )}
      </div>

      {/* Connected agents */}
      {masterAgent && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4, borderTop: '1px solid var(--border)', paddingTop: 6 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
            <Users size={10} color="var(--text-4)" />
            <span style={{ fontSize: 9, fontWeight: 600, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.04em' }}>
              Connected ({connectedAgents.length}/{MAX_CONNECTED})
            </span>
            {connectedAgents.length < MAX_CONNECTED && (
              <button onClick={onAttachConnected} style={{
                marginLeft: 'auto', width: 18, height: 18, borderRadius: 4,
                border: `1px solid ${ACCENT}40`, background: `${ACCENT}08`,
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                cursor: 'pointer', color: ACCENT,
              }}>
                <Plus size={10} />
              </button>
            )}
          </div>

          {connectedAgents.length === 0 && (
            <div style={{ fontSize: 9, color: 'var(--text-4)', fontStyle: 'italic', padding: '2px 0' }}>
              Add agents to run in parallel
            </div>
          )}

          {connectedAgents.map((agent, i) => (
            <div key={`${agent.expertId}-${i}`} style={{
              display: 'flex', alignItems: 'center', gap: 6,
              padding: '3px 8px', borderRadius: 5,
              background: `${ACCENT}06`, border: `1px solid ${ACCENT}20`,
            }}>
              <Bot size={10} color={ACCENT} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <span style={{ fontSize: 10, fontWeight: 600, color: 'var(--text-1)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', display: 'block' }}>
                  {agent.name}
                </span>
              </div>
              <button onClick={() => onDetachConnected(i)} style={{
                width: 16, height: 16, borderRadius: 4, flexShrink: 0,
                border: '1px solid #ef444425', background: 'transparent',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                cursor: 'pointer', color: '#ef4444',
              }}>
                <X size={7} />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export type { MasterAgent };
