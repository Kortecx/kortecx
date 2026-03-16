'use client';

import { useState } from 'react';
import {
  Database, Plus, Play, Download, Sparkles, RefreshCcw,
  CheckCircle2, Clock, AlertTriangle, BarChart3, FileText,
  Zap, Filter,
} from 'lucide-react';
import { DATASETS } from '@/lib/constants';
import type { Dataset } from '@/lib/types';

function fmt(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return `${n}`;
}

function DatasetCard({ ds }: { ds: Dataset }) {
  const statusColor = ds.status === 'ready' ? 'var(--success)'
    : ds.status === 'generating' ? 'var(--amber)'
    : ds.status === 'failed' ? 'var(--error)'
    : 'var(--text-3)';

  const StatusIcon = ds.status === 'ready' ? CheckCircle2
    : ds.status === 'generating' ? RefreshCcw
    : ds.status === 'failed' ? AlertTriangle
    : Clock;

  return (
    <div className="card" style={{ padding: 18 }}>
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12, marginBottom: 14 }}>
        <div style={{
          width: 38, height: 38, borderRadius: 6,
          background: 'var(--teal-dim)',
          border: '1px solid rgba(12,166,120,0.25)',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          flexShrink: 0,
        }}>
          <Database size={17} color="var(--teal)" />
        </div>

        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 3 }}>
            <span style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)' }}>{ds.name}</span>
            <span style={{
              display: 'flex', alignItems: 'center', gap: 4,
              fontSize: 10, fontWeight: 600, color: statusColor,
              textTransform: 'uppercase', letterSpacing: '0.06em',
            }}>
              <StatusIcon size={10} />
              {ds.status}
            </span>
          </div>
          <p style={{ fontSize: 12, color: 'var(--text-3)', margin: 0, lineHeight: 1.4 }}>
            {ds.description}
          </p>
        </div>
      </div>

      {/* Stats */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3,1fr)', gap: 8, marginBottom: 14 }}>
        {[
          { label: 'Samples', value: ds.sampleCount.toLocaleString() },
          { label: 'Size', value: `${(ds.sizeBytes / 1_000_000).toFixed(0)} MB` },
          { label: 'Format', value: ds.format.toUpperCase() },
        ].map(stat => (
          <div key={stat.label} style={{
            padding: '7px 10px',
            background: 'var(--bg)',
            border: '1px solid var(--border)',
            borderRadius: 4,
            textAlign: 'center',
          }}>
            <div className="mono" style={{ fontSize: 12, fontWeight: 700, color: 'var(--text-1)' }}>
              {stat.value}
            </div>
            <div style={{ fontSize: 10, color: 'var(--text-3)', marginTop: 1 }}>{stat.label}</div>
          </div>
        ))}
      </div>

      {/* Quality score */}
      {ds.qualityScore !== undefined && (
        <div style={{ marginBottom: 14 }}>
          <div style={{
            display: 'flex', justifyContent: 'space-between',
            fontSize: 11, color: 'var(--text-3)', marginBottom: 5,
          }}>
            <span>Quality Score</span>
            <span className="mono" style={{
              color: ds.qualityScore >= 90 ? 'var(--success)' : ds.qualityScore >= 75 ? 'var(--warning)' : 'var(--error)',
            }}>
              {ds.qualityScore}%
            </span>
          </div>
          <div className="progress-track">
            <div
              className="progress-fill"
              style={{
                width: `${ds.qualityScore}%`,
                background: ds.qualityScore >= 90 ? 'var(--success)' : ds.qualityScore >= 75 ? 'var(--warning)' : 'var(--error)',
              }}
            />
          </div>
        </div>
      )}

      {/* Categories */}
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, marginBottom: 14 }}>
        {ds.categories.map(cat => (
          <span key={cat} className="badge badge-teal" style={{ fontSize: 10 }}>{cat}</span>
        ))}
        {ds.tags.map(tag => (
          <span key={tag} className="badge badge-neutral" style={{ fontSize: 10 }}>{tag}</span>
        ))}
      </div>

      {/* Actions */}
      <div style={{ display: 'flex', gap: 8 }}>
        <button className="btn btn-primary btn-sm">
          <Play size={12} /> Train Expert
        </button>
        <button className="btn btn-secondary btn-sm">
          <Download size={12} /> Export
        </button>
        <button className="btn btn-ghost btn-sm">
          <BarChart3 size={12} /> Inspect
        </button>
      </div>
    </div>
  );
}

export default function DataSynthesisPage() {
  const [tab, setTab] = useState<'datasets' | 'generate'>('datasets');
  const [synthPrompt, setSynthPrompt] = useState('');
  const [targetCount, setTargetCount] = useState('1000');
  const [format, setFormat] = useState('jsonl');
  const [generating, setGenerating] = useState(false);

  const readyCount     = DATASETS.filter(d => d.status === 'ready').length;
  const generatingCount = DATASETS.filter(d => d.status === 'generating').length;
  const totalSamples   = DATASETS.reduce((s, d) => s + d.sampleCount, 0);

  const handleGenerate = () => {
    if (!synthPrompt.trim()) return;
    setGenerating(true);
    // In production: POST /api/data/synthesize
    setTimeout(() => setGenerating(false), 2000);
  };

  return (
    <div style={{ padding: 24, maxWidth: 1200, margin: '0 auto' }}>

      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        marginBottom: 24,
      }}>
        <div>
          <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
            Data Synthesis
          </h1>
          <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
            Generate, manage, and export high-quality training datasets
          </p>
        </div>
        <button className="btn btn-primary btn-sm" onClick={() => setTab('generate')}>
          <Sparkles size={13} /> Synthesize Data
        </button>
      </div>

      {/* Stats */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4,1fr)', gap: 12, marginBottom: 20 }}>
        {[
          { label: 'TOTAL DATASETS',   value: String(DATASETS.length),      color: 'var(--teal)',    icon: Database },
          { label: 'READY',            value: String(readyCount),            color: 'var(--success)', icon: CheckCircle2 },
          { label: 'GENERATING',       value: String(generatingCount),       color: 'var(--amber)',   icon: RefreshCcw },
          { label: 'TOTAL SAMPLES',    value: fmt(totalSamples),             color: 'var(--primary)', icon: Zap },
        ].map(stat => (
          <div key={stat.label} className="metric-card" style={{ position: 'relative', overflow: 'hidden' }}>
            <div style={{ position: 'absolute', top: 0, left: 0, right: 0, height: 2, background: stat.color, opacity: 0.7 }} />
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
              <div>
                <div className="metric-value">{stat.value}</div>
                <div className="metric-label" style={{ marginTop: 6 }}>{stat.label}</div>
              </div>
              <stat.icon size={16} color={stat.color} />
            </div>
          </div>
        ))}
      </div>

      {/* Tabs */}
      <div style={{ display: 'flex', gap: 0, borderBottom: '1px solid var(--border)', marginBottom: 20 }}>
        {([
          { key: 'datasets', label: 'My Datasets' },
          { key: 'generate', label: 'Generate New' },
        ] as const).map(t => (
          <button
            key={t.key}
            onClick={() => setTab(t.key)}
            style={{
              padding: '10px 18px',
              background: 'none', border: 'none',
              borderBottom: `2px solid ${tab === t.key ? 'var(--teal)' : 'transparent'}`,
              cursor: 'pointer', fontSize: 13,
              fontWeight: tab === t.key ? 600 : 400,
              color: tab === t.key ? 'var(--text-1)' : 'var(--text-3)',
              marginBottom: -1,
            }}
          >
            {t.label}
          </button>
        ))}
      </div>

      {/* Datasets */}
      {tab === 'datasets' && (
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(320px,1fr))', gap: 12 }}>
          {DATASETS.map(ds => <DatasetCard key={ds.id} ds={ds} />)}
        </div>
      )}

      {/* Generate */}
      {tab === 'generate' && (
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 20 }}>
          {/* Main form */}
          <div className="card" style={{ padding: 24 }}>
            <h2 style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)', margin: '0 0 20px' }}>
              Synthesize New Dataset
            </h2>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              <div>
                <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                  Dataset Name
                </label>
                <input className="input" placeholder="e.g. Legal Contract Analysis v2" />
              </div>
              <div>
                <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                  Synthesis Prompt
                </label>
                <textarea
                  className="textarea"
                  style={{ minHeight: 120 }}
                  placeholder="Describe what kind of training data to generate. Be specific about the domain, task type, complexity, and quality requirements..."
                  value={synthPrompt}
                  onChange={e => setSynthPrompt(e.target.value)}
                />
              </div>
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
                <div>
                  <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                    Target Samples
                  </label>
                  <input
                    type="number"
                    className="input"
                    value={targetCount}
                    onChange={e => setTargetCount(e.target.value)}
                    min={100} max={100000}
                  />
                </div>
                <div>
                  <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                    Output Format
                  </label>
                  <select className="input" value={format} onChange={e => setFormat(e.target.value)}>
                    <option value="jsonl">JSONL</option>
                    <option value="alpaca">Alpaca</option>
                    <option value="chatml">ChatML</option>
                    <option value="sharegpt">ShareGPT</option>
                    <option value="csv">CSV</option>
                  </select>
                </div>
              </div>
              <div>
                <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                  Generator Expert
                </label>
                <select className="input">
                  <option>Auto-select (recommended)</option>
                  <option>ResearchPro — for research datasets</option>
                  <option>ContentCraft — for writing datasets</option>
                  <option>CodeForge — for coding datasets</option>
                  <option>DataAnalyst — for analysis datasets</option>
                </select>
              </div>
              <div style={{ display: 'flex', gap: 8, paddingTop: 4 }}>
                <button
                  className="btn btn-primary"
                  onClick={handleGenerate}
                  disabled={!synthPrompt.trim() || generating}
                >
                  {generating
                    ? <><RefreshCcw size={14} className="spin" /> Generating...</>
                    : <><Sparkles size={14} /> Start Synthesis</>
                  }
                </button>
              </div>
            </div>
          </div>

          {/* Tips panel */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
            <div className="card" style={{ padding: 18 }}>
              <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', marginBottom: 12, display: 'flex', alignItems: 'center', gap: 8 }}>
                <Sparkles size={14} color="var(--amber)" /> Synthesis Tips
              </div>
              {[
                { tip: 'Be specific about domain and task type for higher quality outputs' },
                { tip: 'Include example input/output pairs in your prompt for consistency' },
                { tip: 'Start with 500–1000 samples to validate quality before scaling' },
                { tip: 'Use domain-specific experts for specialized datasets' },
              ].map((item, i) => (
                <div key={i} style={{
                  display: 'flex', gap: 8, marginBottom: 8,
                  fontSize: 12, color: 'var(--text-3)', lineHeight: 1.5,
                }}>
                  <span style={{ color: 'var(--teal)', flexShrink: 0, marginTop: 1 }}>›</span>
                  <span>{item.tip}</span>
                </div>
              ))}
            </div>

            <div className="card" style={{ padding: 18 }}>
              <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', marginBottom: 10 }}>
                Estimation
              </div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                {[
                  { label: 'Target samples', value: Number(targetCount).toLocaleString() },
                  { label: 'Est. tokens', value: fmt(Number(targetCount) * 800) },
                  { label: 'Est. cost', value: `$${(Number(targetCount) * 0.0005).toFixed(2)}` },
                  { label: 'Est. time', value: `${Math.ceil(Number(targetCount) / 200)} min` },
                ].map(item => (
                  <div key={item.label} style={{
                    display: 'flex', justifyContent: 'space-between',
                    fontSize: 12,
                  }}>
                    <span style={{ color: 'var(--text-3)' }}>{item.label}</span>
                    <span className="mono" style={{ color: 'var(--text-1)' }}>{item.value}</span>
                  </div>
                ))}
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
