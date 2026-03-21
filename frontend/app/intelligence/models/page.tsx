'use client';

import { useState, useEffect, useCallback } from 'react';
import { motion } from 'framer-motion';
import {
  Boxes, Server, Cloud, ExternalLink, Lock, Search,
  Download, CheckCircle2, Loader2, Trash2, X, HardDrive,
  Sparkles, RefreshCw,
} from 'lucide-react';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';
const KORTECX_CLOUD_URL = 'https://www.kortecx.com';

/* ── Types ────────────────────────────────────────────── */
interface LocalModel {
  name: string;
  size: number;
  modified_at: string;
  digest?: string;
}

/* ── Helpers ──────────────────────────────────────────── */
function formatSize(bytes: number) {
  if (!bytes) return '—';
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(0)} MB`;
  return `${(bytes / 1e3).toFixed(0)} KB`;
}

function timeAgo(iso: string) {
  if (!iso) return '—';
  const sec = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (sec < 60) return 'just now';
  if (sec < 3600) return `${Math.floor(sec / 60)}m ago`;
  if (sec < 86400) return `${Math.floor(sec / 3600)}h ago`;
  return `${Math.floor(sec / 86400)}d ago`;
}

/* ── Tab Type ─────────────────────────────────────────── */
type ModelTab = 'local' | 'kortecx' | 'advanced';

/* ── Page ─────────────────────────────────────────────── */
export default function ModelsPage() {
  const [tab, setTab] = useState<ModelTab>('local');
  const [models, setModels] = useState<LocalModel[]>([]);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState('');
  const [pullModel, setPullModel] = useState('');
  const [pulling, setPulling] = useState<string | null>(null);
  const [pullProgress, setPullProgress] = useState(0);
  const [pullStatus, setPullStatus] = useState('');
  const [engine, setEngine] = useState<'ollama' | 'llamacpp'>('ollama');

  const fetchModels = useCallback(async () => {
    setLoading(true);
    try {
      const resp = await fetch(`${ENGINE_URL}/api/orchestrator/models/${engine}`);
      if (resp.ok) {
        const data = await resp.json();
        setModels(data.models || []);
      }
    } catch { /* engine offline */ }
    setLoading(false);
  }, [engine]);

  // eslint-disable-next-line react-hooks/set-state-in-effect
  useEffect(() => { fetchModels(); }, [fetchModels]);

  const handlePull = async () => {
    if (!pullModel.trim() || pulling) return;
    const name = pullModel.trim();
    setPulling(name);
    setPullProgress(0);
    setPullStatus('Starting...');

    try {
      const resp = await fetch(`${ENGINE_URL}/api/orchestrator/models/pull/stream`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ engine, model: name }),
      });
      if (!resp.ok || !resp.body) { setPullStatus('Failed'); setTimeout(() => setPulling(null), 2000); return; }

      const reader = resp.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';
        for (const line of lines) {
          if (!line.startsWith('data: ')) continue;
          try {
            const data = JSON.parse(line.slice(6));
            if (data.percent !== undefined) setPullProgress(data.percent);
            if (data.status) setPullStatus(data.status);
            if (data.status === 'success') {
              setPullProgress(100);
              await fetchModels();
              setTimeout(() => { setPulling(null); setPullModel(''); }, 1500);
              return;
            }
          } catch { /* skip */ }
        }
      }
      setPullProgress(100);
      await fetchModels();
      setTimeout(() => { setPulling(null); setPullModel(''); }, 1500);
    } catch (err) {
      setPullStatus(`Error: ${err instanceof Error ? err.message : 'Unknown'}`);
      setTimeout(() => setPulling(null), 3000);
    }

    // Log
    fetch('/api/logs', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ level: 'info', message: `Model pulled: ${name}`, source: 'models', metadata: { model: name, engine } }),
    }).catch(() => {});
  };

  const handleDelete = async (name: string) => {
    if (!confirm(`Delete model "${name}"? This cannot be undone.`)) return;
    try {
      await fetch(`${ENGINE_URL}/api/orchestrator/models/delete`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ engine, model: name }),
      });
      await fetchModels();
    } catch { /* ignore */ }
  };

  const filtered = models.filter(m => !search || m.name.toLowerCase().includes(search.toLowerCase()));

  const TABS: Array<{ id: ModelTab; label: string; icon: React.ElementType; color: string; enabled: boolean }> = [
    { id: 'local', label: 'Local Models', icon: HardDrive, color: '#059669', enabled: true },
    { id: 'kortecx', label: 'Kortecx Models', icon: Sparkles, color: '#7C3AED', enabled: false },
    { id: 'advanced', label: 'Advanced Models', icon: Cloud, color: '#2563EB', enabled: false },
  ];

  return (
    <div style={{ padding: 20, maxWidth: '100%' }}>
      {/* Header */}
      <motion.div initial={{ opacity: 0, y: -8 }} animate={{ opacity: 1, y: 0 }}
        style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 20 }}>
        <div>
          <h1 style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', margin: 0, display: 'flex', alignItems: 'center', gap: 8 }}>
            <Boxes size={18} color="#7C3AED" /> Models
          </h1>
          <p style={{ fontSize: 12, color: 'var(--text-3)', margin: '3px 0 0' }}>
            Manage local and cloud model registries
          </p>
        </div>
        <button className="btn btn-secondary btn-sm" onClick={fetchModels} disabled={loading}>
          <RefreshCw size={12} style={loading ? { animation: 'spin 1s linear infinite' } : undefined} /> Refresh
        </button>
      </motion.div>

      {/* Tabs */}
      <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.05 }}
        style={{ display: 'flex', gap: 6, marginBottom: 20 }}>
        {TABS.map(t => (
          <button key={t.id} onClick={() => t.enabled ? setTab(t.id) : window.open(KORTECX_CLOUD_URL, '_blank')}
            style={{
              display: 'flex', alignItems: 'center', gap: 6,
              padding: '8px 16px', borderRadius: 7, fontSize: 12, fontWeight: tab === t.id ? 650 : 450,
              border: `1.5px solid ${tab === t.id ? t.color : 'var(--border-md)'}`,
              background: tab === t.id ? `${t.color}10` : 'transparent',
              color: tab === t.id ? t.color : 'var(--text-3)',
              cursor: 'pointer', transition: 'all 0.15s',
              opacity: t.enabled ? 1 : 0.6,
            }}>
            <t.icon size={13} />
            {t.label}
            {!t.enabled && <Lock size={10} style={{ marginLeft: 2, opacity: 0.6 }} />}
          </button>
        ))}
      </motion.div>

      {/* ── Local Models Tab ──────────────────────────── */}
      {tab === 'local' && (
        <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}>
          {/* Engine + Search + Pull */}
          <div style={{ display: 'flex', gap: 8, marginBottom: 16, flexWrap: 'wrap' }}>
            <select className="input" style={{ width: 120 }} value={engine} onChange={e => setEngine(e.target.value as 'ollama' | 'llamacpp')}>
              <option value="ollama">Ollama</option>
              <option value="llamacpp">llama.cpp</option>
            </select>
            <div style={{ flex: 1, display: 'flex', alignItems: 'center', gap: 6, border: '1px solid var(--border-md)', borderRadius: 4, padding: '0 10px', background: 'var(--bg-surface)' }}>
              <Search size={13} color="var(--text-4)" />
              <input className="input" style={{ border: 'none', padding: '7px 0' }} placeholder="Search models..." value={search} onChange={e => setSearch(e.target.value)} />
            </div>
            <div style={{ display: 'flex', gap: 4 }}>
              <input className="input" style={{ width: 200, fontFamily: 'var(--font-mono)' }} placeholder="Model to pull (e.g. mistral:7b)" value={pullModel} onChange={e => setPullModel(e.target.value)}
                onKeyDown={e => { if (e.key === 'Enter') handlePull(); }} />
              <button className="btn btn-primary btn-sm" onClick={handlePull} disabled={!pullModel.trim() || !!pulling}>
                <Download size={12} /> Pull
              </button>
            </div>
          </div>

          {/* Pull progress */}
          {pulling && (
            <div className="card" style={{ padding: '12px 16px', marginBottom: 16 }}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 6 }}>
                <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-1)', display: 'flex', alignItems: 'center', gap: 6 }}>
                  <Loader2 size={13} style={{ animation: 'spin 1s linear infinite' }} color="#7C3AED" />
                  Pulling {pulling}
                </span>
                <span className="mono" style={{ fontSize: 11, color: pullProgress >= 100 ? '#059669' : 'var(--text-3)' }}>{pullProgress.toFixed(0)}%</span>
              </div>
              <div style={{ height: 4, background: 'var(--border)', borderRadius: 2, overflow: 'hidden' }}>
                <div style={{ height: '100%', width: `${pullProgress}%`, background: pullProgress >= 100 ? '#059669' : '#7C3AED', borderRadius: 2, transition: 'width 0.3s' }} />
              </div>
              <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 4 }}>{pullStatus}</div>
            </div>
          )}

          {/* Model list */}
          <div className="card" style={{ overflow: 'hidden' }}>
            {loading ? (
              <div style={{ padding: 40, textAlign: 'center', color: 'var(--text-3)', fontSize: 12 }}>
                <Loader2 size={18} style={{ margin: '0 auto 8px', animation: 'spin 1s linear infinite', display: 'block' }} />
                Loading models...
              </div>
            ) : filtered.length === 0 ? (
              <div style={{ padding: '40px 20px', textAlign: 'center' }}>
                <Server size={28} color="var(--text-4)" style={{ margin: '0 auto 10px' }} />
                <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-2)', marginBottom: 4 }}>
                  {search ? 'No models match your search' : `No models found on ${engine}`}
                </div>
                <div style={{ fontSize: 11, color: 'var(--text-4)' }}>
                  {search ? 'Try a different search term' : `Is ${engine} running? Pull a model to get started.`}
                </div>
              </div>
            ) : (
              <table className="table-base">
                <thead>
                  <tr>
                    <th>Model</th>
                    <th>Size</th>
                    <th>Modified</th>
                    <th></th>
                  </tr>
                </thead>
                <tbody>
                  {filtered.map((m, i) => (
                    <motion.tr key={m.name} initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: i * 0.02 }}>
                      <td>
                        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                          <Server size={13} color="#059669" />
                          <span className="mono" style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-1)' }}>{m.name}</span>
                        </div>
                      </td>
                      <td><span className="mono" style={{ fontSize: 11 }}>{formatSize(m.size)}</span></td>
                      <td style={{ fontSize: 11, color: 'var(--text-4)' }}>{timeAgo(m.modified_at)}</td>
                      <td>
                        <button onClick={() => handleDelete(m.name)}
                          style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-4)', display: 'flex', padding: 4 }}
                          title="Delete model">
                          <Trash2 size={12} />
                        </button>
                      </td>
                    </motion.tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
          <div style={{ marginTop: 8, fontSize: 10, color: 'var(--text-4)' }}>
            {filtered.length} model{filtered.length !== 1 ? 's' : ''} on {engine}
          </div>
        </motion.div>
      )}

      {/* ── Kortecx Models Tab (Cloud) ───────────────── */}
      {tab === 'kortecx' && (
        <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}
          className="card" style={{ padding: 0, overflow: 'hidden' }}>
          <div style={{
            background: 'linear-gradient(135deg, rgba(124,58,237,0.06) 0%, rgba(236,72,153,0.06) 100%)',
            padding: '48px 32px', textAlign: 'center',
          }}>
            <div style={{ width: 52, height: 52, borderRadius: 12, margin: '0 auto 14px', background: 'rgba(124,58,237,0.1)', border: '1.5px solid rgba(124,58,237,0.2)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
              <Sparkles size={24} color="#7C3AED" />
            </div>
            <h2 style={{ fontSize: 18, fontWeight: 800, color: 'var(--text-1)', margin: '0 0 8px' }}>Kortecx Model Registry</h2>
            <p style={{ fontSize: 13, color: 'var(--text-3)', lineHeight: 1.6, maxWidth: 480, margin: '0 auto 20px' }}>
              Access fine-tuned, optimized models built by the Kortecx team. Includes domain-specific models for coding, research, legal, finance, and more.
            </p>
            <a href={KORTECX_CLOUD_URL} target="_blank" rel="noopener noreferrer"
              className="btn btn-primary" style={{ textDecoration: 'none', display: 'inline-flex', padding: '10px 24px' }}>
              <ExternalLink size={14} /> Sign Up for Kortecx Cloud
            </a>
          </div>
        </motion.div>
      )}

      {/* ── Advanced Models Tab (Cloud) ───────────────── */}
      {tab === 'advanced' && (
        <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}
          className="card" style={{ padding: 0, overflow: 'hidden' }}>
          <div style={{
            background: 'linear-gradient(135deg, rgba(37,99,235,0.06) 0%, rgba(16,185,129,0.06) 100%)',
            padding: '48px 32px', textAlign: 'center',
          }}>
            <div style={{ width: 52, height: 52, borderRadius: 12, margin: '0 auto 14px', background: 'rgba(37,99,235,0.1)', border: '1.5px solid rgba(37,99,235,0.2)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
              <Cloud size={24} color="#2563EB" />
            </div>
            <h2 style={{ fontSize: 18, fontWeight: 800, color: 'var(--text-1)', margin: '0 0 8px' }}>Advanced Models</h2>
            <p style={{ fontSize: 13, color: 'var(--text-3)', lineHeight: 1.6, maxWidth: 480, margin: '0 auto 20px' }}>
              Enterprise-grade models with extended context windows, multi-modal capabilities, and custom training.
              Includes GPT-4o, Claude Opus, Gemini Ultra, and exclusive Kortecx Mixture-of-Experts models.
            </p>
            <a href={KORTECX_CLOUD_URL} target="_blank" rel="noopener noreferrer"
              className="btn btn-primary" style={{ textDecoration: 'none', display: 'inline-flex', padding: '10px 24px' }}>
              <ExternalLink size={14} /> Sign Up for Kortecx Cloud
            </a>
          </div>
        </motion.div>
      )}
    </div>
  );
}
