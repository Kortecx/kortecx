'use client';

import { useApp } from '@/contexts/AppContext';
import { PLATFORMS } from '@/lib/constants';
import { Check, Plus } from 'lucide-react';

export default function PlatformList() {
  const { togglePlatformConnection } = useApp();

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      {PLATFORMS.map(platform => (
        <div
          key={platform.id}
          style={{
            display: 'flex', alignItems: 'center', gap: 12,
            padding: '12px 16px',
            background: platform.connected ? platform.bgColor : 'var(--bg-card)',
            border: `1px solid ${platform.connected ? platform.color + '30' : 'var(--border)'}`,
            borderRadius: 6,
            transition: 'all 0.15s',
          }}
        >
          <span style={{
            width: 10, height: 10, borderRadius: '50%',
            background: platform.color, flexShrink: 0,
          }} />
          <span style={{
            flex: 1, fontSize: 13, fontWeight: 500,
            color: 'var(--text-1)',
          }}>
            {platform.name}
          </span>
          {platform.connected ? (
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <span style={{
                display: 'flex', alignItems: 'center', gap: 4,
                fontSize: 11, color: 'var(--success)',
              }}>
                <Check size={12} /> Connected
              </span>
              <button
                onClick={() => togglePlatformConnection(platform.id)}
                className="btn btn-ghost btn-sm"
                style={{ fontSize: 11, padding: '2px 8px' }}
              >
                Disconnect
              </button>
            </div>
          ) : (
            <button
              onClick={() => togglePlatformConnection(platform.id)}
              className="btn btn-secondary btn-sm"
              style={{ fontSize: 11, padding: '4px 12px' }}
            >
              <Plus size={12} /> Connect
            </button>
          )}
        </div>
      ))}
    </div>
  );
}
