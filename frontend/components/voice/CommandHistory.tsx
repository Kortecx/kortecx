'use client';

import { Clock, CheckCircle2, XCircle, AlertCircle } from 'lucide-react';
import { useApp } from '@/contexts/AppContext';

export default function CommandHistory() {
  const { commandHistory } = useApp();

  if (commandHistory.length === 0) {
    return (
      <div style={{
        padding: 20, textAlign: 'center',
        color: 'var(--text-4)', fontSize: 12,
      }}>
        No voice commands yet. Try speaking a command.
      </div>
    );
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column' }}>
      {commandHistory.map(cmd => (
        <div
          key={cmd.id}
          style={{
            display: 'flex', alignItems: 'flex-start', gap: 10,
            padding: '10px 14px',
            borderBottom: '1px solid var(--border)',
          }}
        >
          <span style={{ flexShrink: 0, marginTop: 2 }}>
            {cmd.status === 'processed' ? (
              <CheckCircle2 size={14} color="var(--success)" />
            ) : cmd.status === 'failed' ? (
              <XCircle size={14} color="#DC2626" />
            ) : (
              <AlertCircle size={14} color="var(--amber)" />
            )}
          </span>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 13, color: 'var(--text-1)', marginBottom: 3 }}>
              {cmd.transcript}
            </div>
            <div style={{
              display: 'flex', alignItems: 'center', gap: 8,
              fontSize: 11, color: 'var(--text-3)',
            }}>
              <span style={{
                textTransform: 'uppercase', fontSize: 10,
                fontWeight: 600, letterSpacing: '0.06em',
                color: 'var(--text-4)',
              }}>
                {cmd.intent}
              </span>
              <span style={{ color: 'var(--text-4)' }}>·</span>
              <span style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
                <Clock size={10} />
                {cmd.timestamp.toLocaleTimeString()}
              </span>
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}
