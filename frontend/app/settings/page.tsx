'use client';

import { useState } from 'react';
import { Settings, Save, RotateCcw } from 'lucide-react';
import { PROVIDERS } from '@/lib/constants';
import type { PlatformSettings, ProviderSlug } from '@/lib/types';

const DEFAULT_SETTINGS: PlatformSettings = {
  tokenBudgetMonthly: 50_000_000,
  defaultProvider: 'anthropic',
  defaultModel: 'claude-sonnet-4-6',
  maxConcurrentAgents: 20,
  loggingLevel: 'info',
  dataRetentionDays: 90,
  webhookUrl: '',
};

export default function SettingsPage() {
  const [settings, setSettings] = useState<PlatformSettings>(DEFAULT_SETTINGS);
  const [saved, setSaved] = useState(false);

  const handleSave = () => {
    setSaved(true);
    setTimeout(() => setSaved(false), 2000);
  };

  const handleReset = () => {
    setSettings(DEFAULT_SETTINGS);
  };

  const selectedProvider = PROVIDERS.find(p => p.slug === settings.defaultProvider);

  return (
    <div style={{ padding: 24, maxWidth: 800, margin: '0 auto' }}>
      {/* Header */}
      <div style={{ marginBottom: 28 }}>
        <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
          Platform Settings
        </h1>
        <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
          Configure your Kortecx platform defaults
        </p>
      </div>

      {/* General Settings */}
      <div className="card" style={{ padding: 24, marginBottom: 16 }}>
        <h2 style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 20px' }}>
          General
        </h2>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
          {/* Default Provider */}
          <div>
            <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
              Default Provider
            </label>
            <select
              className="input"
              style={{ width: '100%' }}
              value={settings.defaultProvider}
              onChange={e => setSettings(s => ({ ...s, defaultProvider: e.target.value as ProviderSlug }))}
            >
              {PROVIDERS.filter(p => p.connected).map(p => (
                <option key={p.id} value={p.slug}>{p.name}</option>
              ))}
            </select>
          </div>

          {/* Default Model */}
          <div>
            <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
              Default Model
            </label>
            <select
              className="input"
              style={{ width: '100%' }}
              value={settings.defaultModel}
              onChange={e => setSettings(s => ({ ...s, defaultModel: e.target.value }))}
            >
              {(selectedProvider?.models ?? []).map(m => (
                <option key={m.id} value={m.id}>{m.name}</option>
              ))}
            </select>
          </div>

          {/* Max Concurrent Agents */}
          <div>
            <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
              Max Concurrent Agents
            </label>
            <input
              className="input"
              type="number"
              style={{ width: '100%' }}
              value={settings.maxConcurrentAgents}
              onChange={e => setSettings(s => ({ ...s, maxConcurrentAgents: parseInt(e.target.value) || 1 }))}
            />
          </div>

          {/* Logging Level */}
          <div>
            <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
              Logging Level
            </label>
            <select
              className="input"
              style={{ width: '100%' }}
              value={settings.loggingLevel}
              onChange={e => setSettings(s => ({ ...s, loggingLevel: e.target.value as PlatformSettings['loggingLevel'] }))}
            >
              <option value="debug">Debug</option>
              <option value="info">Info</option>
              <option value="warn">Warning</option>
              <option value="error">Error</option>
            </select>
          </div>
        </div>
      </div>

      {/* Token & Budget */}
      <div className="card" style={{ padding: 24, marginBottom: 16 }}>
        <h2 style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 20px' }}>
          Tokens & Budget
        </h2>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
          <div>
            <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
              Monthly Token Budget
            </label>
            <input
              className="input"
              type="number"
              style={{ width: '100%' }}
              value={settings.tokenBudgetMonthly}
              onChange={e => setSettings(s => ({ ...s, tokenBudgetMonthly: parseInt(e.target.value) || 0 }))}
            />
            <span style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 2, display: 'block' }}>
              {(settings.tokenBudgetMonthly / 1_000_000).toFixed(0)}M tokens
            </span>
          </div>

          <div>
            <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
              Data Retention (days)
            </label>
            <input
              className="input"
              type="number"
              style={{ width: '100%' }}
              value={settings.dataRetentionDays}
              onChange={e => setSettings(s => ({ ...s, dataRetentionDays: parseInt(e.target.value) || 30 }))}
            />
          </div>
        </div>
      </div>

      {/* Integrations */}
      <div className="card" style={{ padding: 24, marginBottom: 16 }}>
        <h2 style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 20px' }}>
          Integrations
        </h2>

        <div style={{ marginBottom: 16 }}>
          <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
            Webhook URL
          </label>
          <input
            className="input"
            type="url"
            placeholder="https://your-domain.com/webhook"
            style={{ width: '100%' }}
            value={settings.webhookUrl}
            onChange={e => setSettings(s => ({ ...s, webhookUrl: e.target.value }))}
          />
          <span style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 2, display: 'block' }}>
            Receive webhook notifications for task completions, alerts, and training updates
          </span>
        </div>

      </div>

      {/* Actions */}
      <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
        <button className="btn btn-secondary btn-sm" onClick={handleReset}>
          <RotateCcw size={12} /> Reset to Defaults
        </button>
        <button className="btn btn-primary btn-sm" onClick={handleSave}>
          <Save size={12} /> {saved ? 'Saved!' : 'Save Settings'}
        </button>
      </div>
    </div>
  );
}
