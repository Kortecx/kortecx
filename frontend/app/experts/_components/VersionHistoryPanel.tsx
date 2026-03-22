'use client';

import { useState } from 'react';
import { motion } from 'framer-motion';
import { RotateCcw, Clock, Loader2, Check } from 'lucide-react';
import { useExpertVersions } from '@/lib/hooks/useApi';

interface VersionHistoryPanelProps {
  expertId: string;
  filename: string;
  onRestored: () => void;
}

export default function VersionHistoryPanel({ expertId, filename, onRestored }: VersionHistoryPanelProps) {
  const { versions, total, isLoading } = useExpertVersions(expertId, filename);
  const [restoring, setRestoring] = useState<string | null>(null);
  const [restored, setRestored] = useState<string | null>(null);

  const handleRestore = async (versionFilename: string) => {
    setRestoring(versionFilename);
    setRestored(null);
    try {
      const res = await fetch('/api/experts/versions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ expertId, version: versionFilename }),
      });
      if (res.ok) {
        setRestored(versionFilename);
        onRestored();
        setTimeout(() => setRestored(null), 2000);
      }
    } catch (err) {
      console.error('Failed to restore version:', err);
    } finally {
      setRestoring(null);
    }
  };

  return (
    <motion.div
      initial={{ opacity: 0, height: 0 }}
      animate={{ opacity: 1, height: 'auto' }}
      exit={{ opacity: 0, height: 0 }}
      style={{
        borderRadius: 10,
        border: '1px solid var(--border)',
        background: 'var(--bg-elevated)',
        overflow: 'hidden',
      }}
    >
      <div style={{
        padding: '12px 16px',
        borderBottom: '1px solid var(--border)',
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <Clock size={14} color="#D97706" />
          <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>
            Version History: {filename}
          </span>
        </div>
        <span style={{ fontSize: 11, color: 'var(--text-4)' }}>
          {total} version{total !== 1 ? 's' : ''}
        </span>
      </div>

      {isLoading ? (
        <div style={{ padding: '24px 0', textAlign: 'center', color: 'var(--text-4)' }}>
          <Loader2 size={16} className="spin" style={{ margin: '0 auto 6px' }} />
          <div style={{ fontSize: 12 }}>Loading versions...</div>
        </div>
      ) : versions.length === 0 ? (
        <div style={{
          padding: '24px 16px', textAlign: 'center',
          color: 'var(--text-4)', fontSize: 12,
        }}>
          No previous versions found. Versions are created when files are edited.
        </div>
      ) : (
        <div style={{ maxHeight: 240, overflow: 'auto' }}>
          {versions.map((v: { filename: string; date: string; size: number; timestamp: number }) => (
            <div
              key={v.filename}
              style={{
                display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                padding: '10px 16px',
                borderBottom: '1px solid var(--border)',
              }}
            >
              <div>
                <div style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)' }}>
                  {new Date(v.date || v.timestamp).toLocaleString()}
                </div>
                <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 2 }}>
                  {v.size > 1024 ? `${(v.size / 1024).toFixed(1)} KB` : `${v.size} B`}
                </div>
              </div>
              <button
                onClick={() => handleRestore(v.filename)}
                disabled={restoring === v.filename}
                style={{
                  display: 'flex', alignItems: 'center', gap: 4,
                  padding: '5px 10px', borderRadius: 6, fontSize: 11, fontWeight: 500,
                  cursor: restoring === v.filename ? 'wait' : 'pointer',
                  border: restored === v.filename
                    ? '1px solid #10b98140'
                    : '1px solid var(--border-md)',
                  background: restored === v.filename ? '#10b98110' : 'transparent',
                  color: restored === v.filename ? '#10b981' : 'var(--text-3)',
                  transition: 'all 0.15s',
                }}
              >
                {restoring === v.filename ? (
                  <Loader2 size={11} className="spin" />
                ) : restored === v.filename ? (
                  <Check size={11} />
                ) : (
                  <RotateCcw size={11} />
                )}
                {restored === v.filename ? 'Restored' : 'Restore'}
              </button>
            </div>
          ))}
        </div>
      )}
    </motion.div>
  );
}
