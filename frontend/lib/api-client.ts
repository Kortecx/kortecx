/**
 * API Client — centralised fetch wrapper for all Sunday API endpoints.
 * Swap BASE_URL or add authentication headers here for production.
 */

export const API_BASE = '/api';

async function request<T>(
  path: string,
  options?: RequestInit,
): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    headers: { 'Content-Type': 'application/json', ...options?.headers },
    ...options,
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({ message: res.statusText }));
    throw new Error(err.message ?? `API error ${res.status}`);
  }
  return res.json() as Promise<T>;
}

/* ─── Projects ──────────────────────────────────────────────────────────── */
export interface ProjectPayload {
  name: string;
  description?: string;
  platforms?: string[];
  status?: 'active' | 'draft' | 'completed' | 'archived';
}

export const projectsApi = {
  list: (params?: { status?: string; q?: string; range?: string }) => {
    const qs = new URLSearchParams(
      Object.entries(params ?? {}).filter(([, v]) => Boolean(v)) as [string, string][]
    ).toString();
    return request<ProjectRecord[]>(`/projects${qs ? `?${qs}` : ''}`);
  },
  create: (data: ProjectPayload) =>
    request<ProjectRecord>('/projects', { method: 'POST', body: JSON.stringify(data) }),
  update: (id: string, data: Partial<ProjectPayload>) =>
    request<ProjectRecord>(`/projects/${id}`, { method: 'PUT', body: JSON.stringify(data) }),
  remove: (id: string) =>
    request<{ success: boolean }>(`/projects/${id}`, { method: 'DELETE' }),
};

export interface ProjectRecord {
  id: string;
  name: string;
  description?: string;
  createdAt: string;
  updatedAt: string;
  platforms: string[];
  postsCount: number;
  status: 'active' | 'draft' | 'completed' | 'archived';
}

/* ─── Publish ────────────────────────────────────────────────────────────── */
export interface PublishPayload {
  content: string;
  platforms: string[];
  scheduledAt?: string;
  projectId?: string;
}

export interface PublishResult {
  id: string;
  status: 'published' | 'scheduled';
  platforms: string[];
  publishedAt?: string;
  scheduledAt?: string;
}

export const publishApi = {
  publish: (data: PublishPayload) =>
    request<PublishResult>('/publish', { method: 'POST', body: JSON.stringify(data) }),
  scheduled: () => request<ScheduledItem[]>('/publish/scheduled'),
  cancelScheduled: (id: string) =>
    request<{ success: boolean }>(`/publish/scheduled/${id}`, { method: 'DELETE' }),
};

export interface ScheduledItem {
  id: string;
  content: string;
  platforms: string[];
  scheduledAt: string;
  status: 'scheduled' | 'published' | 'failed';
}

/* ─── Analytics ──────────────────────────────────────────────────────────── */
export interface AnalyticsSummary {
  totalReach: number;
  engagements: number;
  postsPublished: number;
  avgEngagement: number;
  reachDelta: number;
  engagementsDelta: number;
  postsDelta: number;
  avgEngagementDelta: number;
}

export const analyticsApi = {
  summary: (range?: '7d' | '30d' | '90d') =>
    request<AnalyticsSummary>(`/analytics${range ? `?range=${range}` : ''}`),
};

/* ─── Platforms ──────────────────────────────────────────────────────────── */
export const platformsApi = {
  list: () => request<PlatformStatus[]>('/platforms'),
  connect: (id: string) =>
    request<PlatformStatus>(`/platforms/${id}/connect`, { method: 'POST' }),
  disconnect: (id: string) =>
    request<PlatformStatus>(`/platforms/${id}/disconnect`, { method: 'POST' }),
};

export interface PlatformStatus {
  id: string;
  connected: boolean;
  username?: string;
  followers?: number;
}

/* ─── Voice ──────────────────────────────────────────────────────────────── */
export interface VoiceCommandPayload {
  transcript: string;
}

export interface VoiceCommandResult {
  intent: string;
  platforms: string[];
  content?: string;
  scheduledAt?: string;
}

export const voiceApi = {
  process: (data: VoiceCommandPayload) =>
    request<VoiceCommandResult>('/voice', { method: 'POST', body: JSON.stringify(data) }),
};
