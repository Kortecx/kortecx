'use client';

import { useState } from 'react';
import { User, Save } from 'lucide-react';

const MOCK_USER = {
  name: '',
  email: '',
  role: 'developer' as const,
  timezone: 'America/Los_Angeles',
  tokenBudgetMonthly: 0,
  tokensUsedThisMonth: 0,
  joinedAt: '',
};

function fmt(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

export default function ProfilePage() {
  const [name, setName] = useState(MOCK_USER.name);
  const [timezone, setTimezone] = useState(MOCK_USER.timezone);
  const [saved, setSaved] = useState(false);
  const [notifications, setNotifications] = useState({
    email: true,
    alertCritical: true,
    alertError: true,
    alertWarning: false,
    alertInfo: false,
    trainingComplete: true,
    weeklyDigest: true,
  });

  const handleSave = () => {
    setSaved(true);
    setTimeout(() => setSaved(false), 2000);
  };

  const usagePct = Math.round((MOCK_USER.tokensUsedThisMonth / MOCK_USER.tokenBudgetMonthly) * 100);

  return (
    <div style={{ padding: 24, maxWidth: 800, margin: '0 auto' }}>
      {/* Header */}
      <div style={{ marginBottom: 28 }}>
        <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
          Profile
        </h1>
        <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
          Manage your account settings
        </p>
      </div>

      {/* User info */}
      <div className="card" style={{ padding: 24, marginBottom: 16 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 16, marginBottom: 20 }}>
          <div style={{
            width: 56, height: 56, borderRadius: '50%',
            background: 'var(--primary-dim)',
            border: '2px solid var(--border-md)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            fontSize: 20, fontWeight: 700, color: 'var(--primary-text)',
          }}>
            {MOCK_USER.name.split(' ').map(n => n[0]).join('')}
          </div>
          <div>
            <div style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)' }}>
              {MOCK_USER.name}
            </div>
            <div style={{ fontSize: 13, color: 'var(--text-3)' }}>{MOCK_USER.email}</div>
            <span className="badge badge-success" style={{ marginTop: 4 }}>
              {MOCK_USER.role}
            </span>
          </div>
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
          <div>
            <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
              Display Name
            </label>
            <input
              className="input"
              value={name}
              onChange={e => setName(e.target.value)}
              style={{ width: '100%' }}
            />
          </div>
          <div>
            <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
              Email
            </label>
            <input className="input" value={MOCK_USER.email} disabled style={{ width: '100%', opacity: 0.6 }} />
          </div>
          <div>
            <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
              Timezone
            </label>
            <select className="input" value={timezone} onChange={e => setTimezone(e.target.value)} style={{ width: '100%' }}>
              <option value="America/Los_Angeles">Pacific Time</option>
              <option value="America/Chicago">Central Time</option>
              <option value="America/New_York">Eastern Time</option>
              <option value="Europe/London">GMT / London</option>
              <option value="Europe/Berlin">CET / Berlin</option>
              <option value="Asia/Tokyo">JST / Tokyo</option>
            </select>
          </div>
          <div>
            <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
              Member Since
            </label>
            <input className="input" value={MOCK_USER.joinedAt} disabled style={{ width: '100%', opacity: 0.6 }} />
          </div>
        </div>
      </div>

      {/* Token Usage */}
      <div className="card" style={{ padding: 24, marginBottom: 16 }}>
        <h2 style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 16px' }}>
          Token Usage
        </h2>
        <div style={{
          display: 'flex', justifyContent: 'space-between',
          fontSize: 12, color: 'var(--text-3)', marginBottom: 6,
        }}>
          <span>{fmt(MOCK_USER.tokensUsedThisMonth)} used this month</span>
          <span className="mono" style={{ color: usagePct > 80 ? 'var(--warning)' : 'var(--text-2)' }}>
            {usagePct}%
          </span>
        </div>
        <div style={{
          height: 6, background: 'var(--bg-elevated)',
          borderRadius: 3, overflow: 'hidden', marginBottom: 6,
        }}>
          <div style={{
            height: '100%',
            width: `${usagePct}%`,
            background: usagePct > 80 ? 'var(--warning)' : 'var(--primary)',
            borderRadius: 3,
          }} />
        </div>
        <div style={{ fontSize: 11, color: 'var(--text-4)' }}>
          Monthly budget: {fmt(MOCK_USER.tokenBudgetMonthly)} tokens
        </div>
      </div>

      {/* Notification Preferences */}
      <div className="card" style={{ padding: 24, marginBottom: 16 }}>
        <h2 style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 16px' }}>
          Notifications
        </h2>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          {[
            { key: 'email', label: 'Email notifications' },
            { key: 'alertCritical', label: 'Critical alerts' },
            { key: 'alertError', label: 'Error alerts' },
            { key: 'alertWarning', label: 'Warning alerts' },
            { key: 'alertInfo', label: 'Info alerts' },
            { key: 'trainingComplete', label: 'Training job completions' },
            { key: 'weeklyDigest', label: 'Weekly usage digest' },
          ].map(item => (
            <label key={item.key} style={{
              display: 'flex', alignItems: 'center', gap: 10,
              cursor: 'pointer', fontSize: 13, color: 'var(--text-2)',
            }}>
              <input
                type="checkbox"
                checked={notifications[item.key as keyof typeof notifications]}
                onChange={e => setNotifications(prev => ({ ...prev, [item.key]: e.target.checked }))}
                style={{ accentColor: '#F04500' }}
              />
              {item.label}
            </label>
          ))}
        </div>
      </div>

      {/* Save */}
      <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
        <button className="btn btn-primary btn-sm" onClick={handleSave}>
          <Save size={12} /> {saved ? 'Saved!' : 'Save Changes'}
        </button>
      </div>
    </div>
  );
}
