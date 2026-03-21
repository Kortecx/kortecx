'use client';

import { useState, useEffect, Suspense } from 'react';
import { useSearchParams } from 'next/navigation';
import useSWR from 'swr';
import { Plus, Play, Clock, BarChart3, Cpu, Loader2 } from 'lucide-react';
import { useTrainingJobs } from '@/lib/hooks/useApi';
import type { TrainingJob, TrainingJobStatus, Dataset, AIProvider } from '@/lib/types';

const fetcher = (url: string) => fetch(url).then(r => {
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
});

function statusBadge(status: TrainingJobStatus) {
  switch (status) {
    case 'training':
      return <span className="badge badge-amber">Training</span>;
    case 'queued':
      return <span className="badge badge-neutral">Queued</span>;
    case 'evaluating':
      return <span className="badge badge-info">Evaluating</span>;
    case 'completed':
      return <span className="badge badge-success">Completed</span>;
    case 'failed':
      return <span className="badge badge-error">Failed</span>;
    case 'preparing':
      return <span className="badge badge-neutral">Preparing</span>;
    default:
      return <span className="badge badge-neutral">{status}</span>;
  }
}

function JobCard({ job, datasets, providers }: { job: TrainingJob; datasets: Dataset[]; providers: AIProvider[] }) {
  const dataset = datasets.find(d => d.id === job.datasetId);
  const allModels = providers.flatMap(p => p.models);
  const baseModel = allModels.find(m => m.id === job.baseModelId);

  return (
    <div className="card" style={{ padding: 20 }}>
      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', marginBottom: 14 }}>
        <div>
          <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)', marginBottom: 3 }}>
            {job.name}
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 11, color: 'var(--text-3)' }}>
            <span>Base: {baseModel?.name ?? job.baseModelId}</span>
            <span style={{ color: 'var(--text-4)' }}>·</span>
            <span>Dataset: {dataset?.name ?? job.datasetId}</span>
          </div>
        </div>
        {statusBadge(job.status)}
      </div>

      {/* Progress */}
      {(job.status === 'training' || job.status === 'evaluating') && (
        <div style={{ marginBottom: 14 }}>
          <div style={{
            display: 'flex', justifyContent: 'space-between',
            fontSize: 11, color: 'var(--text-3)', marginBottom: 4,
          }}>
            <span>Progress</span>
            <span className="mono" style={{ fontWeight: 600, color: 'var(--text-2)' }}>
              {job.progress}%
            </span>
          </div>
          <div className="progress-track">
            <div className="progress-fill" style={{ width: `${job.progress}%` }} />
          </div>
          {job.currentEpoch && (
            <div style={{ fontSize: 11, color: 'var(--text-3)', marginTop: 3 }}>
              Epoch {job.currentEpoch}/{job.epochs}
            </div>
          )}
        </div>
      )}

      {/* Metrics */}
      {job.evalMetrics && (
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3,1fr)', gap: 8, marginBottom: 14 }}>
          <div style={{
            padding: 8, background: 'var(--bg)',
            border: '1px solid var(--border)', borderRadius: 4, textAlign: 'center',
          }}>
            <div className="mono" style={{ fontSize: 14, fontWeight: 700, color: 'var(--text-1)' }}>
              {job.evalMetrics.loss.toFixed(3)}
            </div>
            <div style={{ fontSize: 10, color: 'var(--text-3)', marginTop: 2 }}>Loss</div>
          </div>
          <div style={{
            padding: 8, background: 'var(--bg)',
            border: '1px solid var(--border)', borderRadius: 4, textAlign: 'center',
          }}>
            <div className="mono" style={{ fontSize: 14, fontWeight: 700, color: 'var(--success)' }}>
              {(job.evalMetrics.accuracy * 100).toFixed(1)}%
            </div>
            <div style={{ fontSize: 10, color: 'var(--text-3)', marginTop: 2 }}>Accuracy</div>
          </div>
          <div style={{
            padding: 8, background: 'var(--bg)',
            border: '1px solid var(--border)', borderRadius: 4, textAlign: 'center',
          }}>
            <div className="mono" style={{ fontSize: 14, fontWeight: 700, color: 'var(--text-1)' }}>
              {job.evalMetrics.perplexity?.toFixed(1) ?? '—'}
            </div>
            <div style={{ fontSize: 10, color: 'var(--text-3)', marginTop: 2 }}>Perplexity</div>
          </div>
        </div>
      )}

      {/* Config + Cost */}
      <div style={{
        display: 'flex', gap: 12, fontSize: 11, color: 'var(--text-3)',
        paddingTop: 10, borderTop: '1px solid var(--border)',
      }}>
        <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
          <BarChart3 size={11} /> {job.trainingSamples.toLocaleString()} samples
        </span>
        <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
          <Clock size={11} /> {job.epochs} epochs
        </span>
        {job.gpuHours && (
          <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
            <Cpu size={11} /> {job.gpuHours.toFixed(1)}h GPU
          </span>
        )}
        {job.costUsd && (
          <span className="mono" style={{ marginLeft: 'auto', fontWeight: 600, color: 'var(--text-2)' }}>
            ${job.costUsd.toFixed(2)}
          </span>
        )}
      </div>

      {/* Logs */}
      {job.logs.length > 0 && (
        <div style={{
          marginTop: 12, padding: '8px 10px',
          background: '#0d0d0d', borderRadius: 4,
          maxHeight: 80, overflowY: 'auto',
        }}>
          {job.logs.map((log, i) => (
            <div key={i} className="mono" style={{ fontSize: 10, color: 'rgba(255,255,255,0.6)', lineHeight: 1.6 }}>
              {log}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export default function FinetunePage() {
  return <Suspense><FinetunePageInner /></Suspense>;
}

function FinetunePageInner() {
  const searchParams = useSearchParams();
  const [showNew, setShowNew] = useState(false);

  /* Handle ?action=new → auto-open create dialog */
  useEffect(() => {
    const action = searchParams.get('action');
    if (action !== 'new') return;
    requestAnimationFrame(() => {
      setShowNew(true);
      window.history.replaceState({}, '', '/training/finetune');
    });
  }, [searchParams]);

  const { jobs, isLoading: jobsLoading } = useTrainingJobs() as { jobs: TrainingJob[]; total: number; isLoading: boolean; error: unknown; mutate: () => void };
  const { data: datasetsData, isLoading: datasetsLoading } = useSWR<{ datasets: Dataset[] }>('/api/data/datasets', fetcher);
  const { data: providersData, isLoading: providersLoading } = useSWR<{ providers: AIProvider[] }>('/api/providers', fetcher);

  const datasets = datasetsData?.datasets ?? [];
  const providers = providersData?.providers ?? [];
  const isLoading = jobsLoading || datasetsLoading || providersLoading;

  return (
    <div style={{ padding: 24, maxWidth: 1400, margin: '0 auto' }}>
      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between',
        marginBottom: 24,
      }}>
        <div>
          <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
            Fine-tuning
          </h1>
          <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
            {isLoading ? '...' : `${jobs.length} jobs · ${jobs.filter(j => j.status === 'training').length} active`}
          </p>
        </div>
        <button className="btn btn-primary btn-sm" onClick={() => setShowNew(!showNew)}>
          <Plus size={13} /> New Fine-tune Job
        </button>
      </div>

      {/* New job form */}
      {showNew && (
        <div className="card" style={{ padding: 20, marginBottom: 20 }}>
          <h2 style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 16px' }}>
            Configure Fine-tune Job
          </h2>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
            <div>
              <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                Base Model
              </label>
              <select className="input" style={{ width: '100%' }}>
                {providers.filter(p => p.connected).flatMap(p =>
                  p.models.map(m => (
                    <option key={m.id} value={m.id}>{p.name} — {m.name}</option>
                  ))
                )}
              </select>
            </div>
            <div>
              <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                Training Dataset
              </label>
              <select className="input" style={{ width: '100%' }}>
                {datasets.filter(d => d.status === 'ready').map(d => (
                  <option key={d.id} value={d.id}>{d.name} ({d.sampleCount.toLocaleString()} samples)</option>
                ))}
              </select>
            </div>
            <div>
              <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                Epochs
              </label>
              <input className="input" type="number" defaultValue={5} min={1} max={20} style={{ width: '100%' }} />
            </div>
            <div>
              <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                Learning Rate
              </label>
              <input className="input" type="text" defaultValue="2e-5" style={{ width: '100%' }} />
            </div>
            <div>
              <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                Batch Size
              </label>
              <input className="input" type="number" defaultValue={16} min={1} max={128} style={{ width: '100%' }} />
            </div>
            <div>
              <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                Job Name
              </label>
              <input className="input" type="text" placeholder="My fine-tune job" style={{ width: '100%' }} />
            </div>
          </div>
          <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', marginTop: 16 }}>
            <button className="btn btn-secondary btn-sm" onClick={() => setShowNew(false)}>
              Cancel
            </button>
            <button className="btn btn-primary btn-sm">
              <Play size={12} /> Start Training
            </button>
          </div>
        </div>
      )}

      {/* Jobs grid */}
      {jobsLoading ? (
        <div style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)', fontSize: 13, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8 }}>
          <Loader2 size={16} className="animate-spin" /> Loading fine-tune jobs...
        </div>
      ) : jobs.length === 0 ? (
        <div style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)', fontSize: 13 }}>
          No fine-tune jobs yet. Click &quot;New Fine-tune Job&quot; to create one.
        </div>
      ) : (
        <div style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fill, minmax(400px, 1fr))',
          gap: 12,
        }}>
          {jobs.map(job => (
            <JobCard key={job.id} job={job} datasets={datasets} providers={providers} />
          ))}
        </div>
      )}
    </div>
  );
}
