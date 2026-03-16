'use client';

import { useState, useEffect, useCallback } from 'react';
import { projectsApi, type ProjectRecord, type ProjectPayload } from '@/lib/api-client';

interface UseProjectsOptions {
  status?: string;
  q?: string;
  range?: string;
}

export function useProjects(options: UseProjectsOptions = {}) {
  const [projects, setProjects] = useState<ProjectRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await projectsApi.list(options);
      setProjects(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load projects');
    } finally {
      setLoading(false);
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [options.status, options.q, options.range]);

  useEffect(() => {
    load();
  }, [load]);

  const createProject = useCallback(
    async (payload: ProjectPayload): Promise<ProjectRecord | null> => {
      try {
        const created = await projectsApi.create(payload);
        setProjects((prev) => [created, ...prev]);
        return created;
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to create project');
        return null;
      }
    },
    [],
  );

  const updateProject = useCallback(
    async (id: string, payload: Partial<ProjectPayload>): Promise<ProjectRecord | null> => {
      try {
        const updated = await projectsApi.update(id, payload);
        setProjects((prev) =>
          prev.map((p) => (p.id === id ? updated : p)),
        );
        return updated;
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to update project');
        return null;
      }
    },
    [],
  );

  const deleteProject = useCallback(async (id: string): Promise<boolean> => {
    try {
      await projectsApi.remove(id);
      setProjects((prev) => prev.filter((p) => p.id !== id));
      return true;
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to delete project');
      return false;
    }
  }, []);

  /** Archives a project (status → 'archived') */
  const archiveProject = useCallback(
    async (id: string) => updateProject(id, { status: 'archived' }),
    [updateProject],
  );

  /** Restores an archived project back to 'active' */
  const unarchiveProject = useCallback(
    async (id: string) => updateProject(id, { status: 'active' }),
    [updateProject],
  );

  /** Generic status setter */
  const setProjectStatus = useCallback(
    async (id: string, status: ProjectRecord['status']) => updateProject(id, { status }),
    [updateProject],
  );

  return {
    projects,
    loading,
    error,
    reload: load,
    createProject,
    updateProject,
    deleteProject,
    archiveProject,
    unarchiveProject,
    setProjectStatus,
  };
}

/** Fetches a single project by ID */
export async function fetchProject(id: string): Promise<ProjectRecord | null> {
  try {
    const res = await fetch(`/api/projects/${id}`);
    if (!res.ok) return null;
    return res.json();
  } catch {
    return null;
  }
}
