'use client';

import { useState, useEffect, useCallback } from 'react';
import { motion } from 'framer-motion';
import {
  Sliders, Play, X, Plus,
  Trash2,
} from 'lucide-react';
import { useExperts } from '@/lib/hooks/useApi';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* ── Types ────────────────────────────────────────────── */
interface FineTuneJob {
  id: string;
  name: string;
  expertId?: string;
  baseModel: string;
  engine: string;
  datasetPath: string;
  status: 'queued' | 'preparing' | 'running' | 'completed' | 'failed' | 'cancelled';
  progress: number;
  epochs: number;
  currentEpoch: number;
  learningRate: number;
  batchSize: number;
  createdAt: string;
  error?: string;
}

/* ── Helpers ──────────────────────────────────────────── */
function timeAgo(iso: string) {
  const sec = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (sec < 60) return 'just now';
  if (sec < 3600) return `${Math.floor(sec / 60)}m ago`;
  if (sec < 86400) return `${Math.floor(sec / 3600)}h ago`;
  return `${Math.floor(sec / 86400)}d ago`;
}

const STATUS_STYLE: Record<string, { color: string; bg: string }> = {
  queued:    { color: '#6b7280', bg: '#6b728010' },
  preparing: { color: '#2563EB', bg: '#2563EB10' },
  running:   { color: '#D97706', bg: '#D9770610' },
  completed: { color: '#059669', bg: '#05966910' },
  failed:    { color: '#DC2626', bg: '#DC262610' },
  cancelled: { color: '#6b7280', bg: '#6b728010' },
};

/* ── Page ─────────────────────────────────────────────── */
export default function FineTuningPage() {
  const [jobs, setJobs] = useState<FineTuneJob[]>(() => {
    if (typeof window === 'undefined') return [];
    try {
      const raw = localStorage.getItem('kortecx_finetune_jobs');
      return raw ? JSON.parse(raw) : [];
    } catch { return []; }
  });
  const [showCreate, setShowCreate] = useState(false);
  const { experts } = useExperts();

  // Form state
  const [jobName, setJobName] = useState('');
  const [baseModel, setBaseModel] = useState('llama3.2:3b');
  const [engine, setEngine] = useState('ollama');
  const [datasetPath, setDatasetPath] = useState('');
  const [epochs, setEpochs] = useState(3);
  const [learningRate, setLearningRate] = useState(0.0002);
  const [batchSize, setBatchSize] = useState(4);
  const [expertId, setExpertId] = useState('');

  // Load available models
  const [models, setModels] = useState<Array<{ name: string }>>([]);
  useEffect(() => {
    fetch(`${ENGINE_URL}/api/orchestrator/models/ollama`)
      .then(r => r.ok ? r.json() : { models: [] })
      .then(d => setModels(d.models || []))
      .catch(() => {});
  }, []);

  const saveJobs = useCallback((updated: FineTuneJob[]) => {
    setJobs(updated);
    localStorage.setItem('kortecx_finetune_jobs', JSON.stringify(updated));
  }, []);

  const handleCreate = () => {
    if (!jobName.trim() || !datasetPath.trim()) return;
    const job: FineTuneJob = {
      id: `ft-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
      name: jobName.trim(),
      expertId: expertId || undefined,
      baseModel,
      engine,
      datasetPath: datasetPath.trim(),
      status: 'queued',
      progress: 0,
      epochs,
      currentEpoch: 0,
      learningRate,
      batchSize,
      createdAt: new Date().toISOString(),
    };
    saveJobs([job, ...jobs]);
    setShowCreate(false);
    setJobName('');
    setDatasetPath('');

    // Log to system
    fetch('/api/logs', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        level: 'info',
        message: `Fine-tuning job created: ${job.name} (${baseModel})`,
        source: 'finetuning',
        metadata: { jobId: job.id, model: baseModel, epochs },
      }),
    }).catch(() => {});
  };

  const deleteJob = (id: string) => {
    saveJobs(jobs.filter(j => j.id !== id));
  };

  const LABEL: React.CSSProperties = { fontSize: 11, fontWeight: 600, color: 'var(--text-3)', display: 'block', marginBottom: 4 };

  return (
    <div style={{ padding: 20, maxWidth: '100%' }}>
      {/* Header */}
      <motion.div initial={{ opacity: 0, y: -8 }} animate={{ opacity: 1, y: 0 }}
        style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 20 }}>
        <div>
          <h1 style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', margin: 0, display: 'flex', alignItems: 'center', gap: 8 }}>
            <Sliders size={18} color="#7C3AED" /> Fine-tuning
          </h1>
          <p style={{ fontSize: 12, color: 'var(--text-3)', margin: '3px 0 0' }}>
            Fine-tune local models with LoRA adapters using your datasets
          </p>
        </div>
        <button className="btn btn-primary btn-sm" onClick={() => setShowCreate(true)}>
          <Plus size={13} /> New Job
        </button>
      </motion.div>

      {/* Stats */}
      <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.05 }}
        style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 12, marginBottom: 20 }}>
        {[
          { label: 'Total Jobs', value: jobs.length, color: '#7C3AED' },
          { label: 'Running', value: jobs.filter(j => j.status === 'running').length, color: '#D97706' },
          { label: 'Completed', value: jobs.filter(j => j.status === 'completed').length, color: '#059669' },
          { label: 'Failed', value: jobs.filter(j => j.status === 'failed').length, color: '#DC2626' },
        ].map((s, i) => (
          <motion.div key={s.label} initial={{ opacity: 0, scale: 0.95 }} animate={{ opacity: 1, scale: 1 }} transition={{ delay: 0.05 + i * 0.04 }}
            className="card" style={{ padding: 16, textAlign: 'center' }}>
            <div style={{ fontSize: 24, fontWeight: 800, color: s.color, fontFamily: 'var(--font-mono)' }}>{s.value}</div>
            <div style={{ fontSize: 10, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.06em', fontWeight: 600, marginTop: 4 }}>{s.label}</div>
          </motion.div>
        ))}
      </motion.div>

      {/* Create Dialog */}
      {showCreate && (
        <div style={{ position: 'fixed', inset: 0, background: 'rgba(7,7,26,0.85)', zIndex: 200, display: 'flex', alignItems: 'flex-start', justifyContent: 'center', paddingTop: 60 }}>
          <motion.div initial={{ opacity: 0, scale: 0.96 }} animate={{ opacity: 1, scale: 1 }}
            style={{ width: 560, background: 'var(--bg-surface)', border: '1px solid var(--border-md)', borderRadius: 10, overflow: 'hidden' }}>
            <div style={{ padding: '16px 20px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
              <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>New Fine-tuning Job</div>
              <button onClick={() => setShowCreate(false)} style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-3)', display: 'flex' }}><X size={16} /></button>
            </div>
            <div style={{ padding: 20, display: 'flex', flexDirection: 'column', gap: 14 }}>
              <div>
                <label style={LABEL}>Job Name *</label>
                <input className="input" placeholder="e.g. Customer Support LoRA" value={jobName} onChange={e => setJobName(e.target.value)} />
              </div>
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
                <div>
                  <label style={LABEL}>Base Model</label>
                  <select className="input" value={baseModel} onChange={e => setBaseModel(e.target.value)}>
                    {models.map(m => <option key={m.name} value={m.name}>{m.name}</option>)}
                    {models.length === 0 && <option value="llama3.2:3b">llama3.2:3b</option>}
                  </select>
                </div>
                <div>
                  <label style={LABEL}>Engine</label>
                  <select className="input" value={engine} onChange={e => setEngine(e.target.value)}>
                    <option value="ollama">Ollama</option>
                    <option value="llamacpp">llama.cpp</option>
                  </select>
                </div>
              </div>
              <div>
                <label style={LABEL}>Dataset Path *</label>
                <input className="input" placeholder="/path/to/dataset.jsonl or dataset name" value={datasetPath} onChange={e => setDatasetPath(e.target.value)} />
              </div>
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 12 }}>
                <div>
                  <label style={LABEL}>Epochs</label>
                  <input className="input" type="number" min={1} max={50} value={epochs} onChange={e => setEpochs(parseInt(e.target.value) || 3)} />
                </div>
                <div>
                  <label style={LABEL}>Learning Rate</label>
                  <input className="input" type="number" step={0.0001} min={0.00001} value={learningRate} onChange={e => setLearningRate(parseFloat(e.target.value) || 0.0002)} />
                </div>
                <div>
                  <label style={LABEL}>Batch Size</label>
                  <input className="input" type="number" min={1} max={64} value={batchSize} onChange={e => setBatchSize(parseInt(e.target.value) || 4)} />
                </div>
              </div>
              <div>
                <label style={LABEL}>Target Expert (optional)</label>
                <select className="input" value={expertId} onChange={e => setExpertId(e.target.value)}>
                  <option value="">None — standalone adapter</option>
                  {(experts as Array<{ id: string; name: string }>).map(e => <option key={e.id} value={e.id}>{e.name}</option>)}
                </select>
              </div>
            </div>
            <div style={{ padding: '14px 20px', borderTop: '1px solid var(--border)', display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
              <button className="btn btn-secondary btn-sm" onClick={() => setShowCreate(false)}>Cancel</button>
              <button className="btn btn-primary btn-sm" onClick={handleCreate} disabled={!jobName.trim() || !datasetPath.trim()}>
                <Play size={12} /> Create Job
              </button>
            </div>
          </motion.div>
        </div>
      )}

      {/* Job List */}
      <div className="card" style={{ overflow: 'hidden' }}>
        {jobs.length === 0 ? (
          <div style={{ padding: '60px 20px', textAlign: 'center' }}>
            <Sliders size={32} color="var(--text-4)" style={{ margin: '0 auto 12px' }} />
            <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-2)', marginBottom: 6 }}>No fine-tuning jobs yet</div>
            <div style={{ fontSize: 12, color: 'var(--text-3)', marginBottom: 16 }}>Create a job to fine-tune a local model with your dataset</div>
            <button className="btn btn-primary btn-sm" onClick={() => setShowCreate(true)}>
              <Plus size={12} /> Create First Job
            </button>
          </div>
        ) : (
          <table className="table-base" style={{ width: '100%' }}>
            <thead>
              <tr>
                <th>Job</th>
                <th>Model</th>
                <th>Status</th>
                <th>Progress</th>
                <th>Epochs</th>
                <th>Created</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {jobs.map((job, i) => {
                const st = STATUS_STYLE[job.status] || STATUS_STYLE.queued;
                return (
                  <motion.tr key={job.id} initial={{ opacity: 0, x: -8 }} animate={{ opacity: 1, x: 0 }} transition={{ delay: i * 0.03 }}>
                    <td>
                      <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>{job.name}</div>
                      <div style={{ fontSize: 10, color: 'var(--text-4)', fontFamily: 'var(--font-mono)' }}>{job.id}</div>
                    </td>
                    <td><span className="mono" style={{ fontSize: 11, color: '#7C3AED' }}>{job.baseModel}</span></td>
                    <td>
                      <span style={{ fontSize: 10, fontWeight: 700, padding: '2px 8px', borderRadius: 10, background: st.bg, color: st.color, textTransform: 'uppercase' }}>
                        {job.status}
                      </span>
                    </td>
                    <td>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                        <div style={{ flex: 1, height: 3, background: 'var(--border)', borderRadius: 2, overflow: 'hidden' }}>
                          <div style={{ height: '100%', width: `${job.progress}%`, background: st.color, borderRadius: 2 }} />
                        </div>
                        <span className="mono" style={{ fontSize: 10, color: 'var(--text-4)' }}>{job.progress}%</span>
                      </div>
                    </td>
                    <td style={{ fontSize: 11 }}>{job.currentEpoch}/{job.epochs}</td>
                    <td style={{ fontSize: 11, color: 'var(--text-4)' }}>{timeAgo(job.createdAt)}</td>
                    <td>
                      <button onClick={() => deleteJob(job.id)} style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-4)', display: 'flex' }}>
                        <Trash2 size={12} />
                      </button>
                    </td>
                  </motion.tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
