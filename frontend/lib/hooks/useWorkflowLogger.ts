'use client';

import { useCallback, useRef } from 'react';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

function generateSessionId(): string {
  return `ses-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

/**
 * Hook for logging all workflow builder interactions to the engine's
 * local storage logger. All operations are fire-and-forget (non-blocking).
 */
export function useWorkflowLogger(workflowId: string) {
  const sessionIdRef = useRef(generateSessionId());

  const post = useCallback((endpoint: string, body: Record<string, unknown>) => {
    fetch(`${ENGINE_URL}/api/logs${endpoint}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    }).catch(() => {/* fire and forget */});
  }, []);

  const logInteraction = useCallback((action: string, details?: Record<string, unknown>) => {
    post('/interaction', { workflowId, action, details: details ?? null });
  }, [workflowId, post]);

  const logSessionEvent = useCallback((eventType: string, data?: Record<string, unknown>) => {
    post('/session', { sessionId: sessionIdRef.current, eventType, data: data ?? null });
  }, [post]);

  const saveGoal = useCallback((goalContent: string, source: 'text' | 'file') => {
    post('/goal', { workflowId, goalContent, source });
  }, [workflowId, post]);

  const saveConfig = useCallback((config: Record<string, unknown>) => {
    post('/config', { workflowId, config });
  }, [workflowId, post]);

  const logMetricsConfig = useCallback((metricsConfig: Record<string, unknown>) => {
    post('/metrics', { workflowId, metricsConfig });
  }, [workflowId, post]);

  const saveTags = useCallback((tags: string[]) => {
    post('/tags', { workflowId, tags });
  }, [workflowId, post]);

  const savePermissions = useCallback((permissions: Record<string, unknown>) => {
    post('/permissions', { workflowId, permissions });
  }, [workflowId, post]);

  const logStepChange = useCallback((action: string, stepData: Record<string, unknown>) => {
    post('/step', { workflowId, action, stepData });
  }, [workflowId, post]);

  return {
    sessionId: sessionIdRef.current,
    logInteraction,
    logSessionEvent,
    saveGoal,
    saveConfig,
    logMetricsConfig,
    saveTags,
    savePermissions,
    logStepChange,
  };
}
