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

/* ── Logs ───────────────────────────────────────────── */
export function useLogs(level?: string, limit = 100) {
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

/* ── Workflows ──────────────────────────────────────── */
export function useWorkflows(templatesOnly = false) {
  const url = templatesOnly ? '/api/workflows?templates=1' : '/api/workflows';
  const { data, error, isLoading, mutate } = useSWR(
    url,
    fetcher,
    { refreshInterval: 30_000 },
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
    { refreshInterval: 10_000 },
  );
  return {
    runs:     data?.runs  ?? [],
    total:    data?.total ?? 0,
    error,
    isLoading,
    mutate,
  };
}

/* ── Training Jobs ──────────────────────────────────── */
export function useTrainingJobs(status?: string) {
  const params = status ? `?status=${status}` : '';
  const { data, error, isLoading, mutate } = useSWR(
    `/api/training${params}`,
    fetcher,
    { refreshInterval: 10_000 },
  );
  return {
    jobs:     data?.jobs  ?? [],
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
