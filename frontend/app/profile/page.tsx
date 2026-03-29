'use client';

import { useState, useEffect, useCallback } from 'react';
import Image from 'next/image';
import { motion } from 'framer-motion';
import {
  fadeUp, fadeDown, stagger, hoverLift, rowEntrance,
  buttonHover, progressBar, emptyState,
} from '@/lib/motion';
import {
  User, Save, Activity, Key, Link2, Shield, RefreshCw,
  Copy, Trash2, Plus, CheckCircle2, AlertCircle,
  BarChart3, Cpu, Zap, Database, Globe, Clock,
} from 'lucide-react';

/* ── Types ─────────────────────────────────────────────── */
interface PlatformMetrics {
  totalWorkflowRuns: number;
  successfulRuns: number;
  failedRuns: number;
  totalTokensUsed: number;
  totalTokenBudget: number;
  activeTasks: number;
  queuedTasks: number;
  avgLatencyMs: number;
  engineStatus: 'online' | 'offline' | 'unknown';
}

interface Provider {
  id: string;
  name: string;
  slug: string;
  expertCount: number;
  totalRuns: number;
  avgLatency: number;
  connected: boolean;
}

interface SocialConnection {
  id: string;
  platform: string;
  platformUsername: string;
  platformAvatar: string | null;
  status: string;
  scopes: string | null;
  lastUsedAt: string | null;
  createdAt: string;
}

interface ApiKeyEntry {
  id: string;
  name: string;
  prefix: string;
  createdAt: string;
  lastUsedAt: string | null;
}

/* ── Helpers ───────────────────────────────────────────── */
const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

function fmt(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

function timeAgo(dateStr: string | null): string {
  if (!dateStr) return 'Never';
  const diff = Date.now() - new Date(dateStr).getTime();
  const mins = Math.floor(diff / 60_000);
  if (mins < 1) return 'Just now';
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

const LABEL: React.CSSProperties = {
  fontSize: 12, fontWeight: 500, color: 'var(--text-2)',
  display: 'block', marginBottom: 4,
};

/* ── Profile Page ──────────────────────────────────────── */
export default function ProfilePage() {
  const [metrics, setMetrics] = useState<PlatformMetrics | null>(null);
  const [providers, setProviders] = useState<Provider[]>([]);
  const [connections, setConnections] = useState<SocialConnection[]>([]);
  const [apiKeys, setApiKeys] = useState<ApiKeyEntry[]>([]);
  const [newKeyName, setNewKeyName] = useState('');
  const [newKeyValue, setNewKeyValue] = useState<string | null>(null);
  const [showNewKey, setShowNewKey] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Profile fields
  const [displayName, setDisplayName] = useState('');
  const [timezone, setTimezone] = useState(Intl.DateTimeFormat().resolvedOptions().timeZone);

  const fetchData = useCallback(async () => {
    setLoading(true);
    setError(null);

    try {
      const [metricsRes, providersRes, connectionsRes, keysRes] = await Promise.allSettled([
        fetch('/api/metrics'),
        fetch('/api/providers'),
        fetch('/api/oauth/connections'),
        fetch('/api/oauth/credentials'),
      ]);

      // Metrics
      if (metricsRes.status === 'fulfilled' && metricsRes.value.ok) {
        const data = await metricsRes.value.json();
        setMetrics({
          totalWorkflowRuns: data.runs?.total ?? data.totalRuns ?? 0,
          successfulRuns: data.runs?.successful ?? data.successfulRuns ?? 0,
          failedRuns: data.runs?.failed ?? data.failedRuns ?? 0,
          totalTokensUsed: data.tokens?.total ?? data.totalTokensUsed ?? 0,
          totalTokenBudget: data.tokens?.budget ?? data.totalTokenBudget ?? 50_000_000,
          activeTasks: data.tasks?.active ?? data.activeTasks ?? 0,
          queuedTasks: data.tasks?.queued ?? data.queuedTasks ?? 0,
          avgLatencyMs: data.avgLatencyMs ?? 0,
          engineStatus: 'online',
        });
      } else {
        setMetrics({
          totalWorkflowRuns: 0, successfulRuns: 0, failedRuns: 0,
          totalTokensUsed: 0, totalTokenBudget: 50_000_000,
          activeTasks: 0, queuedTasks: 0, avgLatencyMs: 0,
          engineStatus: 'unknown',
        });
      }

      // Providers
      if (providersRes.status === 'fulfilled' && providersRes.value.ok) {
        const data = await providersRes.value.json();
        setProviders(data.providers ?? data ?? []);
      }

      // Social connections
      if (connectionsRes.status === 'fulfilled' && connectionsRes.value.ok) {
        const data = await connectionsRes.value.json();
        setConnections(data.connections ?? data ?? []);
      }

      // API keys
      if (keysRes.status === 'fulfilled' && keysRes.value.ok) {
        const data = await keysRes.value.json();
        setApiKeys(data.keys ?? data ?? []);
      }

      // Check engine health
      try {
        const engineRes = await fetch(`${ENGINE_URL}/health`, { signal: AbortSignal.timeout(3000) });
        if (engineRes.ok) {
          setMetrics(prev => prev ? { ...prev, engineStatus: 'online' } : prev);
        }
      } catch {
        setMetrics(prev => prev ? { ...prev, engineStatus: 'offline' } : prev);
      }
    } catch (_err) {
      setError('Failed to load profile data');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { fetchData(); }, [fetchData]);

  // Load saved profile from localStorage
  useEffect(() => {
    try {
      const raw = localStorage.getItem('kortecx_profile');
      if (raw) {
        const p = JSON.parse(raw);
        if (p.displayName) setDisplayName(p.displayName);
        if (p.timezone) setTimezone(p.timezone);
      }
    } catch { /* ignore */ }
  }, []);

  const handleSaveProfile = () => {
    localStorage.setItem('kortecx_profile', JSON.stringify({ displayName, timezone }));
    setSaved(true);
    setTimeout(() => setSaved(false), 2000);
  };

  const handleCreateKey = async () => {
    if (!newKeyName.trim()) return;
    try {
      const res = await fetch('/api/oauth/credentials', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: newKeyName }),
      });
      if (res.ok) {
        const data = await res.json();
        setNewKeyValue(data.key ?? data.apiKey ?? null);
        setShowNewKey(true);
        setNewKeyName('');
        fetchData();
      }
    } catch { /* ignore */ }
  };

  const handleDeleteKey = async (id: string) => {
    try {
      await fetch(`/api/oauth/credentials?id=${id}`, { method: 'DELETE' });
      setApiKeys(prev => prev.filter(k => k.id !== id));
    } catch { /* ignore */ }
  };

  const tokenPct = metrics ? Math.min(100, Math.round((metrics.totalTokensUsed / metrics.totalTokenBudget) * 100)) : 0;
  const successRate = metrics && metrics.totalWorkflowRuns > 0
    ? Math.round((metrics.successfulRuns / metrics.totalWorkflowRuns) * 100)
    : 0;

  return (
    <div style={{ padding: 24, maxWidth: 900, margin: '0 auto' }}>
      {/* Header */}
      <motion.div
        variants={fadeDown}
        initial="hidden"
        animate="show"
        style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 24 }}
      >
        <div>
          <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0, display: 'flex', alignItems: 'center', gap: 8 }}>
            <User size={18} color="var(--text-3)" /> Profile
          </h1>
          <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
            Platform usage, connections, and API key management
          </p>
        </div>
        <button className="btn btn-secondary btn-sm" onClick={fetchData} disabled={loading}>
          <RefreshCw size={12} className={loading ? 'spin' : ''} /> Refresh
        </button>
      </motion.div>

      {error && (
        <div style={{ padding: '10px 14px', borderRadius: 6, background: 'rgba(239,68,68,0.1)', border: '1px solid rgba(239,68,68,0.2)', color: '#ef4444', fontSize: 12, marginBottom: 16, display: 'flex', alignItems: 'center', gap: 8 }}>
          <AlertCircle size={14} /> {error}
        </div>
      )}

      {/* Stagger container for all section cards */}
      <motion.div variants={stagger(0.1)} initial="hidden" animate="show">

        {/* ── User Info Card ──────────────────────────────── */}
        <motion.div
          className="card"
          style={{ padding: 24, marginBottom: 16 }}
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.38, ease: [0.25, 0.46, 0.45, 0.94] }}
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: 16, marginBottom: 20 }}>
            <div style={{
              width: 56, height: 56, borderRadius: '50%',
              background: 'var(--primary-dim)',
              border: '2px solid var(--border-md)',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              fontSize: 20, fontWeight: 700, color: 'var(--primary-text)',
            }}>
              {displayName ? displayName.split(' ').map(n => n[0]).join('').slice(0, 2).toUpperCase() : <User size={22} />}
            </div>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)' }}>
                {displayName || 'Kortecx User'}
              </div>
              <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 2, display: 'flex', alignItems: 'center', gap: 6 }}>
                <Globe size={11} /> {timezone}
              </div>
            </div>
            <div style={{
              padding: '4px 10px', borderRadius: 4, fontSize: 11, fontWeight: 600,
              background: metrics?.engineStatus === 'online' ? 'rgba(5,150,105,0.1)' : 'rgba(239,68,68,0.1)',
              color: metrics?.engineStatus === 'online' ? '#059669' : '#ef4444',
            }}>
              {metrics?.engineStatus === 'online' ? 'Engine Online' : 'Engine Offline'}
            </div>
          </div>

          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
            <div>
              <label style={LABEL}>Display Name</label>
              <input className="input" value={displayName} onChange={e => setDisplayName(e.target.value)} placeholder="Your name" style={{ width: '100%' }} />
            </div>
            <div>
              <label style={LABEL}>Timezone</label>
              <select className="input" value={timezone} onChange={e => setTimezone(e.target.value)} style={{ width: '100%' }}>
                <option value="America/Los_Angeles">Pacific Time</option>
                <option value="America/Chicago">Central Time</option>
                <option value="America/New_York">Eastern Time</option>
                <option value="America/Denver">Mountain Time</option>
                <option value="Europe/London">GMT / London</option>
                <option value="Europe/Berlin">CET / Berlin</option>
                <option value="Europe/Paris">CET / Paris</option>
                <option value="Asia/Tokyo">JST / Tokyo</option>
                <option value="Asia/Shanghai">CST / Shanghai</option>
                <option value="Asia/Kolkata">IST / India</option>
                <option value="Australia/Sydney">AEST / Sydney</option>
                <option value="Pacific/Auckland">NZST / Auckland</option>
              </select>
            </div>
          </div>
          <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 14 }}>
            <motion.button
              className="btn btn-primary btn-sm"
              onClick={handleSaveProfile}
              {...buttonHover}
            >
              {saved ? <><CheckCircle2 size={12} /> Saved!</> : <><Save size={12} /> Save Profile</>}
            </motion.button>
          </div>
        </motion.div>

        {/* ── Platform Usage Stats ────────────────────────── */}
        <motion.div className="card" style={{ padding: 24, marginBottom: 16 }} variants={fadeUp}>
          <h2 style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 16px', display: 'flex', alignItems: 'center', gap: 8 }}>
            <BarChart3 size={15} color="#7C3AED" /> Platform Usage
          </h2>
          <motion.div
            variants={stagger(0.08)}
            initial="hidden"
            animate="show"
            style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 12, marginBottom: 16 }}
          >
            {[
              { label: 'Total Runs', value: fmt(metrics?.totalWorkflowRuns ?? 0), icon: Zap, color: '#D97706' },
              { label: 'Success Rate', value: `${successRate}%`, icon: CheckCircle2, color: '#059669' },
              { label: 'Active Tasks', value: String(metrics?.activeTasks ?? 0), icon: Activity, color: '#2563EB' },
              { label: 'Avg Latency', value: `${Math.round(metrics?.avgLatencyMs ?? 0)}ms`, icon: Clock, color: '#7C3AED' },
            ].map(stat => (
              <motion.div
                key={stat.label}
                variants={fadeUp}
                {...hoverLift}
                style={{
                  padding: 14, borderRadius: 8,
                  background: 'var(--bg)',
                  border: '1px solid var(--border)',
                }}
              >
                <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 8 }}>
                  <stat.icon size={13} color={stat.color} />
                  <span style={{ fontSize: 11, color: 'var(--text-3)', fontWeight: 500 }}>{stat.label}</span>
                </div>
                <div className="mono" style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)' }}>
                  {stat.value}
                </div>
              </motion.div>
            ))}
          </motion.div>

          {/* Token usage bar */}
          <div style={{ padding: 14, background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 8 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 12, color: 'var(--text-2)', marginBottom: 6 }}>
              <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <Database size={12} /> Tokens Used
              </span>
              <span className="mono" style={{ color: tokenPct > 80 ? 'var(--warning)' : 'var(--text-2)' }}>
                {fmt(metrics?.totalTokensUsed ?? 0)} / {fmt(metrics?.totalTokenBudget ?? 0)} ({tokenPct}%)
              </span>
            </div>
            <div style={{ height: 6, background: 'var(--bg-elevated)', borderRadius: 3, overflow: 'hidden' }}>
              <motion.div
                {...progressBar(tokenPct)}
                style={{
                  height: '100%',
                  background: tokenPct > 80 ? 'var(--warning)' : '#7C3AED',
                  borderRadius: 3,
                }}
              />
            </div>
          </div>
        </motion.div>

        {/* ── Connected Providers ──────────────────────────── */}
        <motion.div className="card" style={{ padding: 24, marginBottom: 16 }} variants={fadeUp}>
          <h2 style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 16px', display: 'flex', alignItems: 'center', gap: 8 }}>
            <Cpu size={15} color="#2563EB" /> Connected Providers
          </h2>
          {providers.length === 0 ? (
            <motion.div
              {...emptyState}
              style={{ fontSize: 12, color: 'var(--text-3)', padding: '16px 0', textAlign: 'center' }}
            >
              No providers configured. Add API keys in Settings to connect.
            </motion.div>
          ) : (
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(200px, 1fr))', gap: 10 }}>
              {providers.map((p, index) => (
                <motion.div
                  key={p.id || p.slug}
                  {...rowEntrance(index)}
                  style={{
                    padding: 12, borderRadius: 8,
                    background: 'var(--bg)',
                    border: `1px solid ${p.connected ? 'rgba(5,150,105,0.3)' : 'var(--border)'}`,
                  }}
                >
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
                    <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>{p.name}</span>
                    <span style={{
                      fontSize: 10, fontWeight: 600, padding: '2px 6px', borderRadius: 3,
                      background: p.connected ? 'rgba(5,150,105,0.1)' : 'rgba(107,114,128,0.1)',
                      color: p.connected ? '#059669' : 'var(--text-4)',
                    }}>
                      {p.connected ? 'Connected' : 'Not Set'}
                    </span>
                  </div>
                  <div style={{ display: 'flex', gap: 16, fontSize: 11, color: 'var(--text-3)' }}>
                    <span>{p.expertCount ?? 0} experts</span>
                    <span>{fmt(p.totalRuns ?? 0)} runs</span>
                  </div>
                </motion.div>
              ))}
            </div>
          )}
        </motion.div>

        {/* ── Social Connections ───────────────────────────── */}
        <motion.div className="card" style={{ padding: 24, marginBottom: 16 }} variants={fadeUp}>
          <h2 style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 16px', display: 'flex', alignItems: 'center', gap: 8 }}>
            <Link2 size={15} color="#EC4899" /> Connected Platforms
          </h2>
          {connections.length === 0 ? (
            <motion.div
              {...emptyState}
              style={{ fontSize: 12, color: 'var(--text-3)', padding: '16px 0', textAlign: 'center' }}
            >
              No social platforms connected. Connect via the Integrations page.
            </motion.div>
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              {connections.map((conn, index) => (
                <motion.div
                  key={conn.id}
                  {...rowEntrance(index)}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 12,
                    padding: '10px 14px', borderRadius: 8,
                    background: 'var(--bg)', border: '1px solid var(--border)',
                  }}
                >
                  {conn.platformAvatar ? (
                    <Image src={conn.platformAvatar} alt="" width={28} height={28} style={{ borderRadius: '50%' }} />
                  ) : (
                    <div style={{ width: 28, height: 28, borderRadius: '50%', background: 'var(--primary-dim)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                      <Globe size={14} color="var(--text-3)" />
                    </div>
                  )}
                  <div style={{ flex: 1 }}>
                    <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', textTransform: 'capitalize' }}>
                      {conn.platform}
                    </div>
                    <div style={{ fontSize: 11, color: 'var(--text-3)' }}>
                      @{conn.platformUsername}
                    </div>
                  </div>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 10, fontSize: 11 }}>
                    <span style={{
                      padding: '2px 6px', borderRadius: 3, fontWeight: 600,
                      background: conn.status === 'active' ? 'rgba(5,150,105,0.1)' : 'rgba(239,68,68,0.1)',
                      color: conn.status === 'active' ? '#059669' : '#ef4444',
                    }}>
                      {conn.status}
                    </span>
                    <span style={{ color: 'var(--text-4)' }}>
                      {timeAgo(conn.lastUsedAt)}
                    </span>
                  </div>
                </motion.div>
              ))}
            </div>
          )}
        </motion.div>

        {/* ── API Key Management ──────────────────────────── */}
        <motion.div className="card" style={{ padding: 24, marginBottom: 16 }} variants={fadeUp}>
          <h2 style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 16px', display: 'flex', alignItems: 'center', gap: 8 }}>
            <Key size={15} color="#D97706" /> API Keys
          </h2>

          {/* Create new key */}
          <div style={{ display: 'flex', gap: 8, marginBottom: 16 }}>
            <input
              className="input"
              placeholder="Key name (e.g. CI/CD, CLI tool)"
              value={newKeyName}
              onChange={e => setNewKeyName(e.target.value)}
              onKeyDown={e => e.key === 'Enter' && handleCreateKey()}
              style={{ flex: 1 }}
            />
            <motion.button {...buttonHover} className="btn btn-primary btn-sm" onClick={handleCreateKey} disabled={!newKeyName.trim()}>
              <Plus size={12} /> Create Key
            </motion.button>
          </div>

          {/* Newly created key display */}
          {newKeyValue && showNewKey && (
            <div style={{
              padding: 12, borderRadius: 8, marginBottom: 16,
              background: 'rgba(5,150,105,0.06)', border: '1px solid rgba(5,150,105,0.2)',
            }}>
              <div style={{ fontSize: 11, color: '#059669', fontWeight: 600, marginBottom: 6, display: 'flex', alignItems: 'center', gap: 6 }}>
                <Shield size={12} /> Copy this key now — it will not be shown again
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <code className="mono" style={{ flex: 1, fontSize: 12, color: 'var(--text-1)', padding: '6px 10px', background: 'var(--bg)', borderRadius: 4, border: '1px solid var(--border)' }}>
                  {newKeyValue}
                </code>
                <button className="btn btn-secondary btn-sm" onClick={() => { navigator.clipboard.writeText(newKeyValue); }}>
                  <Copy size={12} /> Copy
                </button>
                <button className="btn btn-secondary btn-sm" onClick={() => { setShowNewKey(false); setNewKeyValue(null); }}>
                  Dismiss
                </button>
              </div>
            </div>
          )}

          {/* Existing keys list */}
          {apiKeys.length === 0 ? (
            <motion.div
              {...emptyState}
              style={{ fontSize: 12, color: 'var(--text-3)', padding: '12px 0', textAlign: 'center' }}
            >
              No API keys created yet. Create one to access the platform API.
            </motion.div>
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              {apiKeys.map((key, index) => (
                <motion.div
                  key={key.id}
                  {...rowEntrance(index)}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 12,
                    padding: '10px 14px', borderRadius: 8,
                    background: 'var(--bg)', border: '1px solid var(--border)',
                  }}
                >
                  <Key size={13} color="var(--text-4)" />
                  <div style={{ flex: 1 }}>
                    <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-1)' }}>{key.name}</div>
                    <div style={{ fontSize: 11, color: 'var(--text-4)' }}>
                      <code className="mono">{key.prefix}...****</code>
                      <span style={{ marginLeft: 8 }}>Created {timeAgo(key.createdAt)}</span>
                      {key.lastUsedAt && <span style={{ marginLeft: 8 }}>Last used {timeAgo(key.lastUsedAt)}</span>}
                    </div>
                  </div>
                  <button
                    className="btn btn-secondary btn-sm"
                    onClick={() => handleDeleteKey(key.id)}
                    style={{ color: '#ef4444' }}
                  >
                    <Trash2 size={12} /> Revoke
                  </button>
                </motion.div>
              ))}
            </div>
          )}
        </motion.div>

      </motion.div>
    </div>
  );
}
