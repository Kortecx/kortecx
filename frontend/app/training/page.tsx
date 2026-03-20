'use client';

import { useState, useEffect, Suspense } from 'react';
import { useSearchParams } from 'next/navigation';
import useSWR from 'swr';
import {
  Brain, Play, Pause, Plus, Trash2, ChevronDown, ChevronUp,
  Clock, Zap, BarChart3, CheckCircle2, AlertTriangle,
  Database, Cpu, TrendingUp, RefreshCcw, Download, Loader2,
} from 'lucide-react';
import { useTrainingJobs, useExperts } from '@/lib/hooks/useApi';
import type { TrainingJob, Expert, Dataset } from '@/lib/types';

const fetcher = (url: string) => fetch(url).then(r => {
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
});

function jobStatusBadge(status: TrainingJob['status']) {
  switch (status) {
    case 'training':   return <span className="badge badge-amber">Training</span>;
    case 'queued':     return <span className="badge badge-neutral">Queued</span>;
    case 'completed':  return <span className="badge badge-success">Completed</span>;
    case 'failed':     return <span className="badge badge-error">Failed</span>;
    case 'cancelled':  return <span className="badge badge-neutral">Cancelled</span>;
    case 'preparing':  return <span className="badge badge-info">Preparing</span>;
    case 'evaluating': return <span className="badge badge-info">Evaluating</span>;
    default:           return <span className="badge badge-neutral">{status}</span>;
  }
}

function fmt(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

function timeAgo(iso: string) {
  const diff = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

function JobCard({ job, experts, datasets }: { job: TrainingJob; experts: Expert[]; datasets: Dataset[] }) {
  const [expanded, setExpanded] = useState(false);
  const expert = experts.find(e => e.id === job.expertId);
  const dataset = datasets.find(d => d.id === job.datasetId);
  const isActive = job.status === 'training' || job.status === 'preparing' || job.status === 'evaluating';

  return (
    <div className="card" style={{ overflow: 'hidden' }}>
      {/* Header */}
      <div style={{ padding: '16px 18px', borderBottom: '1px solid var(--border)' }}>
        <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
          {/* Icon */}
          <div style={{
            width: 38, height: 38, borderRadius: 6,
            background: isActive ? 'var(--amber-dim)' : 'var(--bg-elevated)',
            border: `1px solid ${isActive ? 'rgba(230,119,0,0.3)' : 'var(--border)'}`,
            display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0,
          }}>
            <Brain size={17} color={isActive ? 'var(--amber)' : 'var(--text-3)'} />
          </div>

          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
              <span style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)' }}>{job.name}</span>
              {jobStatusBadge(job.status)}
            </div>
            <div style={{ display: 'flex', gap: 10, fontSize: 12, color: 'var(--text-3)' }}>
              {expert && <span>Expert: <span style={{ color: 'var(--text-2)' }}>{expert.name}</span></span>}
              {dataset && <span>Dataset: <span style={{ color: 'var(--text-2)' }}>{dataset.name}</span></span>}
              <span>Created {timeAgo(job.createdAt)}</span>
            </div>
          </div>

          {/* Actions */}
          <div style={{ display: 'flex', gap: 6, flexShrink: 0 }}>
            {job.status === 'training' && (
              <button className="btn btn-secondary btn-sm">
                <Pause size={12} /> Pause
              </button>
            )}
            {(job.status === 'completed') && (
              <button className="btn btn-secondary btn-sm">
                <Download size={12} /> Export
              </button>
            )}
            <button
              className="btn btn-ghost btn-icon btn-sm"
              onClick={() => setExpanded(v => !v)}
            >
              {expanded ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
            </button>
          </div>
        </div>

        {/* Progress */}
        {(isActive || job.status === 'queued') && (
          <div style={{ marginTop: 14 }}>
            <div style={{
              display: 'flex', justifyContent: 'space-between',
              fontSize: 11, color: 'var(--text-3)', marginBottom: 6,
            }}>
              <span>
                {job.currentEpoch !== undefined
                  ? `Epoch ${job.currentEpoch}/${job.epochs}`
                  : job.status === 'queued' ? 'Waiting for resources' : 'Initializing'
                }
              </span>
              <span className="mono">{job.progress}%</span>
            </div>
            <div className="progress-track">
              <div
                className="progress-fill"
                style={{
                  width: `${job.progress}%`,
                  background: isActive ? 'var(--amber)' : 'var(--text-4)',
                }}
              />
            </div>
            {job.evalMetrics && (
              <div style={{
                display: 'flex', gap: 16, marginTop: 10, fontSize: 12,
              }}>
                <span style={{ color: 'var(--text-3)' }}>
                  Loss: <span className="mono" style={{ color: 'var(--text-1)' }}>{job.evalMetrics.loss.toFixed(3)}</span>
                </span>
                <span style={{ color: 'var(--text-3)' }}>
                  Accuracy: <span className="mono" style={{ color: 'var(--success)' }}>
                    {(job.evalMetrics.accuracy * 100).toFixed(1)}%
                  </span>
                </span>
                {job.evalMetrics.perplexity && (
                  <span style={{ color: 'var(--text-3)' }}>
                    Perplexity: <span className="mono" style={{ color: 'var(--text-2)' }}>
                      {job.evalMetrics.perplexity.toFixed(1)}
                    </span>
                  </span>
                )}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Expanded details */}
      {expanded && (
        <div style={{ padding: '14px 18px', borderBottom: '1px solid var(--border)' }}>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4,1fr)', gap: 12, marginBottom: 16 }}>
            {[
              { label: 'Base Model', value: job.baseModelId, icon: Cpu },
              { label: 'Training Samples', value: fmt(job.trainingSamples), icon: Database },
              { label: 'Learning Rate', value: job.learningRate.toExponential(0), icon: TrendingUp },
              { label: 'Batch Size', value: String(job.batchSize), icon: Zap },
            ].map(item => (
              <div key={item.label} style={{
                padding: '10px 12px',
                background: 'var(--bg)',
                border: '1px solid var(--border)',
                borderRadius: 4,
              }}>
                <div style={{ fontSize: 10, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.06em', marginBottom: 4 }}>
                  {item.label}
                </div>
                <div className="mono" style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>
                  {item.value}
                </div>
              </div>
            ))}
          </div>

          {/* Logs */}
          {job.logs.length > 0 && (
            <div style={{
              background: 'var(--bg)',
              border: '1px solid var(--border)',
              borderRadius: 4,
              padding: '10px 12px',
            }}>
              <div style={{
                fontSize: 10, color: 'var(--text-3)', textTransform: 'uppercase',
                letterSpacing: '0.06em', marginBottom: 8,
              }}>
                Recent Logs
              </div>
              {job.logs.map((log, i) => (
                <div key={i} className="mono" style={{
                  fontSize: 11, color: 'var(--text-2)', marginBottom: 4,
                  display: 'flex', gap: 8,
                }}>
                  <span style={{ color: 'var(--text-4)' }}>[{i + 1}]</span>
                  <span>{log}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export default function TrainingLabPage() {
  return <Suspense><TrainingLabPageInner /></Suspense>;
}

function TrainingLabPageInner() {
  const searchParams = useSearchParams();
  const [tab, setTab] = useState<'jobs' | 'datasets' | 'new'>('jobs');

  /* Handle ?action=new → auto-switch to New tab */
  useEffect(() => {
    if (searchParams.get('action') === 'new') {
      setTab('new');
      window.history.replaceState({}, '', '/training');
    }
  }, [searchParams]);

  const { jobs, isLoading: jobsLoading } = useTrainingJobs() as { jobs: TrainingJob[]; total: number; isLoading: boolean; error: unknown; mutate: () => void };
  const { experts, isLoading: expertsLoading } = useExperts() as { experts: Expert[]; total: number; isLoading: boolean; error: unknown; mutate: () => void };
  const { data: datasetsData, isLoading: datasetsLoading } = useSWR<{ datasets: Dataset[] }>('/api/data/datasets', fetcher);
  const datasets = datasetsData?.datasets ?? [];

  const isLoading = jobsLoading || expertsLoading || datasetsLoading;

  const activeJobs    = jobs.filter(j => j.status === 'training' || j.status === 'preparing');
  const queuedJobs    = jobs.filter(j => j.status === 'queued');
  const completedJobs = jobs.filter(j => j.status === 'completed');

  return (
    <div style={{ padding: 24, maxWidth: 1200, margin: '0 auto' }}>

      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        marginBottom: 24,
      }}>
        <div>
          <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
            Training Lab
          </h1>
          <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
            Fine-tune and train specialized expert agents on your data
          </p>
        </div>
        <button
          className="btn btn-primary btn-sm"
          onClick={() => setTab('new')}
        >
          <Plus size={13} /> New Training Job
        </button>
      </div>

      {/* Stats */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4,1fr)', gap: 12, marginBottom: 20 }}>
        {[
          { label: 'ACTIVE JOBS',    value: String(activeJobs.length),     color: 'var(--amber)',   icon: Brain },
          { label: 'QUEUED',         value: String(queuedJobs.length),      color: 'var(--text-3)', icon: Clock },
          { label: 'COMPLETED',      value: String(completedJobs.length),   color: 'var(--success)', icon: CheckCircle2 },
          { label: 'DATASETS READY', value: String(datasets.filter(d => d.status === 'ready').length), color: 'var(--teal)', icon: Database },
        ].map(stat => (
          <div key={stat.label} className="metric-card" style={{ position: 'relative', overflow: 'hidden' }}>
            <div style={{ position: 'absolute', top: 0, left: 0, right: 0, height: 2, background: stat.color, opacity: 0.7 }} />
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
              <div>
                <div className="metric-value">{isLoading ? '...' : stat.value}</div>
                <div className="metric-label" style={{ marginTop: 6 }}>{stat.label}</div>
              </div>
              <stat.icon size={16} color={stat.color} />
            </div>
          </div>
        ))}
      </div>

      {/* Tabs */}
      <div style={{ display: 'flex', gap: 0, marginBottom: 20, borderBottom: '1px solid var(--border)' }}>
        {(['jobs', 'datasets', 'new'] as const).map(t => (
          <button
            key={t}
            onClick={() => setTab(t)}
            style={{
              padding: '10px 18px',
              background: 'none',
              border: 'none',
              borderBottom: `2px solid ${tab === t ? 'var(--primary)' : 'transparent'}`,
              cursor: 'pointer',
              fontSize: 13,
              fontWeight: tab === t ? 600 : 400,
              color: tab === t ? 'var(--text-1)' : 'var(--text-3)',
              transition: 'color 0.15s',
              textTransform: 'capitalize',
              marginBottom: -1,
            }}
          >
            {t === 'jobs' ? 'Training Jobs' : t === 'datasets' ? 'Datasets' : 'New Job'}
          </button>
        ))}
      </div>

      {/* Jobs tab */}
      {tab === 'jobs' && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          {jobsLoading ? (
            <div style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)', fontSize: 13, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8 }}>
              <Loader2 size={16} className="animate-spin" /> Loading training jobs...
            </div>
          ) : jobs.length === 0 ? (
            <div style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)', fontSize: 13 }}>
              No training jobs yet. Create one to get started.
            </div>
          ) : (
            jobs.map(job => <JobCard key={job.id} job={job} experts={experts} datasets={datasets} />)
          )}
        </div>
      )}

      {/* Datasets tab */}
      {tab === 'datasets' && (
        <div>
          <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: 14 }}>
            <button className="btn btn-primary btn-sm">
              <Plus size={13} /> New Dataset
            </button>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            {datasetsLoading ? (
              <div style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)', fontSize: 13, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8 }}>
                <Loader2 size={16} className="animate-spin" /> Loading datasets...
              </div>
            ) : datasets.length === 0 ? (
              <div style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)', fontSize: 13 }}>
                No datasets yet. Upload or generate a dataset to begin training.
              </div>
            ) : (
              datasets.map(ds => (
                <div key={ds.id} className="card" style={{ padding: '14px 18px' }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                    <div style={{
                      width: 36, height: 36, borderRadius: 6,
                      background: 'var(--teal-dim)',
                      border: '1px solid rgba(12,166,120,0.25)',
                      display: 'flex', alignItems: 'center', justifyContent: 'center',
                      flexShrink: 0,
                    }}>
                      <Database size={15} color="var(--teal)" />
                    </div>

                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 3 }}>
                        <span style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)' }}>{ds.name}</span>
                        <span className={`badge ${ds.status === 'ready' ? 'badge-success' : ds.status === 'generating' ? 'badge-amber' : 'badge-neutral'}`}>
                          {ds.status}
                        </span>
                      </div>
                      <div style={{ fontSize: 12, color: 'var(--text-3)' }}>{ds.description}</div>
                      <div style={{ display: 'flex', gap: 12, marginTop: 4, fontSize: 11, color: 'var(--text-3)' }}>
                        <span className="mono">{ds.sampleCount.toLocaleString()} samples</span>
                        <span className="mono">{(ds.sizeBytes / 1_000_000).toFixed(0)} MB</span>
                        <span className="badge badge-neutral" style={{ fontSize: 10 }}>{ds.format.toUpperCase()}</span>
                        {ds.qualityScore && (
                          <span>Quality: <span style={{ color: 'var(--success)' }}>{ds.qualityScore}%</span></span>
                        )}
                      </div>
                    </div>

                    <div style={{ display: 'flex', gap: 6, flexShrink: 0 }}>
                      <button className="btn btn-secondary btn-sm">
                        <Download size={12} /> Export
                      </button>
                      <button className="btn btn-primary btn-sm">
                        <Play size={12} /> Train
                      </button>
                    </div>
                  </div>
                </div>
              ))
            )}
          </div>
        </div>
      )}

      {/* New Job tab */}
      {tab === 'new' && (
        <div className="card" style={{ padding: 24, maxWidth: 700 }}>
          <h2 style={{ fontSize: 16, fontWeight: 700, color: 'var(--text-1)', marginBottom: 20, marginTop: 0 }}>
            Create Training Job
          </h2>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
            <div>
              <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                Job Name
              </label>
              <input className="input" placeholder="e.g. ResearchPro v3.0 — Legal Domain" />
            </div>
            <div>
              <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                Target Expert
              </label>
              <select className="input">
                <option value="">Create new expert</option>
                {experts.map(e => <option key={e.id} value={e.id}>{e.name} (fine-tune)</option>)}
              </select>
            </div>
            <div>
              <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                Base Model
              </label>
              <select className="input">
                <option>claude-sonnet-4-6</option>
                <option>claude-haiku-4-5</option>
                <option>gpt-4o</option>
                <option>o3-mini</option>
              </select>
            </div>
            <div>
              <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                Training Dataset
              </label>
              <select className="input">
                {datasets.filter(d => d.status === 'ready').map(d => (
                  <option key={d.id} value={d.id}>{d.name} ({d.sampleCount.toLocaleString()} samples)</option>
                ))}
              </select>
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 12 }}>
              <div>
                <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>Epochs</label>
                <input type="number" className="input" defaultValue={5} min={1} max={20} />
              </div>
              <div>
                <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>Learning Rate</label>
                <input className="input" defaultValue="2e-5" />
              </div>
              <div>
                <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>Batch Size</label>
                <input type="number" className="input" defaultValue={16} />
              </div>
            </div>
            <div style={{ display: 'flex', gap: 10, paddingTop: 8 }}>
              <button className="btn btn-primary">
                <Play size={14} /> Start Training
              </button>
              <button className="btn btn-secondary" onClick={() => setTab('jobs')}>
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
