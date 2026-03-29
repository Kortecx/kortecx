'use client';

import useSWR from 'swr';

const fetcher = (url: string) => fetch(url).then(r => {
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
});

/* ── Tasks ──────────────────────────────────────────── */
export function useTasks(status?: string, limit = 100) {
  const params = new URLSearchParams();
  if (status) params.set('status', status);
  params.set('limit', String(limit));
  const { data, error, isLoading, mutate } = useSWR(
    `/api/tasks?${params}`,
    fetcher,
    { refreshInterval: 5_000 },
  );
  return {
    tasks:     data?.tasks ?? [],
    total:     data?.total ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── Metrics ────────────────────────────────────────── */
export function useMetrics() {
  const { data, error, isLoading, mutate } = useSWR(
    '/api/metrics',
    fetcher,
    { refreshInterval: 10_000 },
  );
  return {
    metrics:  data?.current  ?? null,
    hourly:   data?.hourly   ?? [],
    totals:   data?.totals   ?? null,
    error,
    isLoading,
    mutate,
  };
}

/* ── Alerts ─────────────────────────────────────────── */
export function useAlerts(unackOnly = false) {
  const url = unackOnly ? '/api/alerts?unack=1' : '/api/alerts';
  const { data, error, isLoading, mutate } = useSWR(
    url,
    fetcher,
    { refreshInterval: 15_000 },
  );
  return {
    alerts:   data?.alerts ?? [],
    error,
    isLoading,
    mutate,
  };
}

/* ── Alert Rules ───────────────────────────────────── */
export function useAlertRules() {
  const { data, error, isLoading, mutate } = useSWR(
    '/api/alerts/rules',
    fetcher,
    { refreshInterval: 30_000 },
  );
  return {
    rules:    data?.rules ?? [],
    error,
    isLoading,
    mutate,
  };
}

/* ── Logs ───────────────────────────────────────────── */
export function useLogs(level?: string, limit = 500) {
  const params = new URLSearchParams();
  if (level) params.set('level', level);
  params.set('limit', String(limit));
  const { data, error, isLoading, mutate } = useSWR(
    `/api/logs?${params}`,
    fetcher,
    { refreshInterval: 5_000 },
  );
  return {
    logs:     data?.logs ?? [],
    error,
    isLoading,
    mutate,
  };
}

/* ── Monitoring (combined) ──────────────────────────── */
export function useMonitoring() {
  const { data, error, isLoading, mutate } = useSWR(
    '/api/monitoring',
    fetcher,
    { refreshInterval: 8_000 },
  );
  return {
    system:            data?.system  ?? null,
    alerts:            data?.alerts  ?? [],
    logs:              data?.logs    ?? [],
    unackedAlertCount: data?.unackedAlertCount ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── Experts ────────────────────────────────────────── */
export function useExperts(status?: string) {
  const params = status ? `?status=${status}` : '';
  const { data, error, isLoading, mutate } = useSWR(
    `/api/experts${params}`,
    fetcher,
    { refreshInterval: 20_000 },
  );
  return {
    experts:  data?.experts ?? [],
    total:    data?.total   ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── PRISM Graph (version-based efficient polling) ─── */
export function usePrismGraph() {
  // Poll version cheaply every 10s
  const { data: versionData } = useSWR(
    '/api/experts/graph/version',
    (url: string) => fetch(url).then(r => r.ok ? r.json() : null).catch(() => null),
    { refreshInterval: 10_000 },
  );
  const versionKey = versionData?.version ?? null;

  // Only fetch full edges when version changes (or on first load)
  const { data, error, isLoading, mutate } = useSWR(
    versionKey !== null ? `/api/experts/graph?v=${versionKey}` : '/api/experts/graph',
    fetcher,
    { refreshInterval: 0, revalidateOnFocus: false },
  );
  return {
    edges: data?.edges ?? [],
    total: data?.total ?? 0,
    version: data?.version ?? null,
    error,
    isLoading,
    mutate,
  };
}

/* ── Marketplace Graph (Qdrant-based, with auto-embed on first load) ── */
export function useMarketplaceGraph() {
  const { data, error, isLoading, mutate } = useSWR(
    '/api/experts/graph?source=marketplace',
    fetcher,
    { refreshInterval: 0, revalidateOnFocus: false },
  );
  return {
    edges: data?.edges ?? [],
    total: data?.total ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── Plans ─────────────────────────────────────────── */
export function usePlans(workflowId?: string) {
  const url = workflowId ? `/api/plans?workflowId=${workflowId}` : '/api/plans';
  const { data, error, isLoading, mutate } = useSWR(url, fetcher);
  return {
    plans: data?.plans ?? [],
    total: data?.total ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── Workflows ──────────────────────────────────────── */
export function useWorkflows(templatesOnly = false) {
  const url = templatesOnly ? '/api/workflows?templates=1' : '/api/workflows';
  const { data, error, isLoading, mutate } = useSWR(
    url,
    fetcher,
    { refreshInterval: 5_000 },
  );
  return {
    workflows: data?.workflows ?? [],
    total:     data?.total     ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── Workflow Runs ──────────────────────────────────── */
export function useWorkflowRuns(workflowId?: string, limit = 50) {
  const params = new URLSearchParams();
  if (workflowId) params.set('workflowId', workflowId);
  params.set('limit', String(limit));
  const { data, error, isLoading, mutate } = useSWR(
    `/api/workflows/runs?${params}`,
    fetcher,
    { refreshInterval: 5_000 },
  );
  return {
    runs:     data?.runs  ?? [],
    total:    data?.total ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── Agents ─────────────────────────────────────────── */
export function useAgents() {
  const { data, error, isLoading, mutate } = useSWR(
    '/api/agents',
    fetcher,
    { refreshInterval: 5_000 },
  );
  return {
    agents:  data?.agents  ?? [],
    total:   data?.total   ?? 0,
    active:  data?.active  ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── HF Datasets (tracked) ─────────────────────────── */
export function useHfDatasets() {
  const { data, error, isLoading, mutate } = useSWR(
    '/api/datasets',
    fetcher,
    { refreshInterval: 10_000 },
  );
  return {
    datasets: data?.datasets ?? [],
    total:    data?.total ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── Datasets (local synthesized) ──────────────────── */
export function useDatasets() {
  const { data, error, isLoading, mutate } = useSWR(
    '/api/data/datasets',
    fetcher,
    { refreshInterval: 30_000 },
  );
  return {
    datasets: data?.datasets ?? [],
    total:    data?.total ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── Providers (with DB connection status) ─────────── */
export function useProviders() {
  const { data, error, isLoading, mutate } = useSWR(
    '/api/providers',
    fetcher,
    { refreshInterval: 60_000 },
  );
  return {
    providers: data?.providers ?? [],
    error,
    isLoading,
    mutate,
  };
}

/* ── Synthesis Jobs ────────────────────────────────── */
export function useSynthesisJobs() {
  const { data, error, isLoading, mutate } = useSWR(
    '/api/synthesis',
    fetcher,
    { refreshInterval: 5_000 },
  );
  return {
    jobs:     data?.jobs ?? [],
    total:    data?.total ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── Analytics ─────────────────────────────────────── */
export function useAnalytics() {
  const { data, error, isLoading, mutate } = useSWR(
    '/api/analytics',
    fetcher,
    { refreshInterval: 60_000 },
  );
  return {
    analytics: data ?? null,
    error,
    isLoading,
    mutate,
  };
}

/* ── Live Metrics (from engine) ───────────────────── */
export function useLiveMetrics() {
  const { data, error, isLoading, mutate } = useSWR(
    `${process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000'}/api/metrics/live`,
    fetcher,
    { refreshInterval: 5_000 },
  );
  return {
    metrics: data ?? null,
    error,
    isLoading,
    mutate,
  };
}

/* ── Metrics History ──────────────────────────────── */
export function useMetricsHistory(limit = 100) {
  const { data, error, isLoading, mutate } = useSWR(
    `${process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000'}/api/metrics/history?limit=${limit}`,
    fetcher,
    { refreshInterval: 30_000 },
  );
  return {
    snapshots: data?.snapshots ?? [],
    total:     data?.total ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── Run Audit Trail ──────────────────────────────── */
export function useRunAudit(runId: string | null) {
  const { data, error, isLoading, mutate } = useSWR(
    runId ? `${process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000'}/api/metrics/runs/${runId}/audit` : null,
    fetcher,
  );
  return {
    operations: data?.operations ?? [],
    total:      data?.total ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── Expert Performance Stats ─────────────────────── */
export function useExpertStats() {
  const { data, error, isLoading, mutate } = useSWR(
    `${process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000'}/api/metrics/experts/stats`,
    fetcher,
    { refreshInterval: 30_000 },
  );
  return {
    experts: data?.experts ?? [],
    error,
    isLoading,
    mutate,
  };
}

/* ── Step Executions ──────────────────────────────── */
export function useStepExecutions(runId: string | null) {
  const { data, error, isLoading, mutate } = useSWR(
    runId ? `/api/workflows/executions?runId=${runId}` : null,
    fetcher,
    { refreshInterval: 3_000 },
  );
  return {
    executions: data?.executions ?? [],
    total:      data?.total ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── Expert Runs ─────────────────────────────────────── */
export function useExpertRuns(status?: string, limit = 50) {
  const params = new URLSearchParams();
  if (status) params.set('status', status);
  params.set('limit', String(limit));
  const { data, error, isLoading, mutate } = useSWR(
    `/api/experts/run?${params}`,
    fetcher,
    { refreshInterval: 5_000 },
  );
  return {
    runs:     data?.runs ?? [],
    total:    data?.total ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── Expert Files ────────────────────────────────────── */
export function useExpertFiles(expertId: string | null) {
  const { data, error, isLoading, mutate } = useSWR(
    expertId ? `/api/experts/files?expertId=${expertId}` : null,
    fetcher,
  );
  return {
    files:    data?.files ?? [],
    error,
    isLoading,
    mutate,
  };
}

/* ── Expert Versions ─────────────────────────────────── */
export function useExpertVersions(expertId: string | null, filename: string | null) {
  const { data, error, isLoading, mutate } = useSWR(
    expertId && filename
      ? `/api/experts/versions?expertId=${expertId}&filename=${encodeURIComponent(filename)}`
      : null,
    fetcher,
  );
  return {
    versions: data?.versions ?? [],
    total:    data?.total ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── Embeddings Collections ───────────────────────────── */
export function useEmbeddingCollections() {
  const { data, error, isLoading, mutate } = useSWR(
    '/api/embeddings?action=collections',
    fetcher,
    { refreshInterval: 30_000 },
  );
  return {
    collections: data?.collections ?? (data?.name ? [data] : []),
    error,
    isLoading,
    mutate,
  };
}
