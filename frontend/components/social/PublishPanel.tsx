'use client';

import { X, Send, Clock, FileText } from 'lucide-react';
import { useApp } from '@/contexts/AppContext';
import { PLATFORMS } from '@/lib/constants';

export default function PublishPanel() {
  const {
    publishPanelOpen,
    setPublishPanelOpen,
    generatedContent,
    selectedPublishPlatforms,
    togglePublishPlatform,
  } = useApp();

  if (!publishPanelOpen) return null;

  return (
    <div style={{
      position: 'fixed', top: 0, right: 0, bottom: 0,
      width: 420, background: 'var(--bg-surface)',
      borderLeft: '1px solid var(--border)',
      zIndex: 50, display: 'flex', flexDirection: 'column',
      boxShadow: '-4px 0 24px rgba(0,0,0,0.08)',
    }}>
      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        padding: '14px 20px', borderBottom: '1px solid var(--border)',
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <Send size={15} color="var(--primary)" />
          <span style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)' }}>
            Publish Content
          </span>
        </div>
        <button
          onClick={() => setPublishPanelOpen(false)}
          style={{
            background: 'none', border: 'none', cursor: 'pointer',
            color: 'var(--text-3)', padding: 4, borderRadius: 4,
          }}
        >
          <X size={16} />
        </button>
      </div>

      {/* Content */}
      <div style={{ padding: 20, flex: 1, overflowY: 'auto' }}>
        {generatedContent ? (
          <div style={{
            padding: 16, background: 'var(--bg)',
            border: '1px solid var(--border)', borderRadius: 6,
            marginBottom: 20,
          }}>
            <div style={{
              display: 'flex', alignItems: 'center', gap: 6,
              marginBottom: 10, fontSize: 11, color: 'var(--text-3)',
              fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.06em',
            }}>
              <FileText size={12} /> Generated Content
            </div>
            <p style={{
              fontSize: 13, color: 'var(--text-1)',
              lineHeight: 1.6, margin: 0,
            }}>
              {generatedContent.content}
            </p>
          </div>
        ) : (
          <div style={{
            padding: 40, textAlign: 'center',
            color: 'var(--text-4)', fontSize: 13,
          }}>
            No content generated yet. Use voice or workflow to generate content.
          </div>
        )}

        {/* Platform selection */}
        <div style={{ marginBottom: 20 }}>
          <div style={{
            fontSize: 12, fontWeight: 600, color: 'var(--text-2)',
            marginBottom: 10,
          }}>
            Select platforms
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            {PLATFORMS.map(p => {
              const selected = selectedPublishPlatforms.includes(p.id);
              return (
                <button
                  key={p.id}
                  onClick={() => togglePublishPlatform(p.id)}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 10,
                    padding: '8px 12px',
                    background: selected ? p.bgColor : 'transparent',
                    border: `1px solid ${selected ? p.color + '40' : 'var(--border)'}`,
                    borderRadius: 5, cursor: 'pointer',
                    fontSize: 13, textAlign: 'left',
                    color: selected ? p.color : 'var(--text-2)',
                    fontWeight: selected ? 500 : 400,
                    transition: 'all 0.12s',
                  }}
                >
                  <span style={{
                    width: 8, height: 8, borderRadius: '50%',
                    background: p.color, flexShrink: 0,
                  }} />
                  {p.name}
                </button>
              );
            })}
          </div>
        </div>
      </div>

      {/* Footer */}
      <div style={{
        padding: '14px 20px', borderTop: '1px solid var(--border)',
        display: 'flex', gap: 8,
      }}>
        <button className="btn btn-secondary btn-sm" style={{ flex: 1, justifyContent: 'center' }}>
          <Clock size={13} /> Schedule
        </button>
        <button
          className="btn btn-primary btn-sm"
          style={{ flex: 1, justifyContent: 'center' }}
          disabled={selectedPublishPlatforms.length === 0}
        >
          <Send size={13} /> Publish Now
        </button>
      </div>
    </div>
  );
}
