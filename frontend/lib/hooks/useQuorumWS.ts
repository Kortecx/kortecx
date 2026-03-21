'use client';

import { useState, useEffect, useCallback, useRef } from 'react';
import type {
  QuorumRunResult,
  QuorumMetricsSnapshot,
  QuorumSubmitRequest,
  QuorumEventType,
} from '@/lib/types';

// ── Agent live state ──────────────────────────────────

interface QuorumAgentLive {
  agentId: string;
  role: 'master' | 'worker';
  subtask?: string;
  model?: string;
  status: 'created' | 'thinking' | 'success' | 'failed' | 'recovered';
  tokensUsed?: number;
  durationMs?: number;
  contentPreview?: string;
  error?: string;
  attempt?: number;
  originalAgent?: string;
}

// ── Phase live state ──────────────────────────────────

interface QuorumPhaseLive {
  phase: string;
  status: 'started' | 'complete';
  detail?: string;
  wallClockMs?: number;
  speedup?: number;
  parallel?: boolean;
  subtasks?: string[];
}

// ── Quorum event log entry ────────────────────────────

interface QuorumLogEntry {
  event: QuorumEventType;
  data: Record<string, unknown>;
  timestamp: string;
}

// ── Top-level quorum state ────────────────────────────

interface QuorumLiveState {
  runId: string | null;
  status: 'idle' | 'connecting' | 'queued' | 'running' | 'complete' | 'failed' | 'cancelled';
  phase: QuorumPhaseLive | null;
  agents: Record<string, QuorumAgentLive>;
  metrics: QuorumMetricsSnapshot | null;
  result: QuorumRunResult | null;
  error: string | null;
  events: QuorumLogEntry[];
  subtasks: string[];
}

const ENGINE_WS_URL = process.env.NEXT_PUBLIC_ENGINE_WS_URL || 'ws://localhost:8000/ws';

export function useQuorumWS() {
  const wsRef = useRef<WebSocket | null>(null);
  const [state, setState] = useState<QuorumLiveState>({
    runId: null,
    status: 'idle',
    phase: null,
    agents: {},
    metrics: null,
    result: null,
    error: null,
    events: [],
    subtasks: [],
  });

  const handleEvent = useCallback((raw: string) => {
    try {
      const msg = JSON.parse(raw);
      const event = msg.event as QuorumEventType;
      const data = msg.data || {};

      const logEntry: QuorumLogEntry = {
        event,
        data,
        timestamp: msg.timestamp || new Date().toISOString(),
      };

      setState(prev => {
        const next = { ...prev, events: [...prev.events, logEntry] };

        switch (event) {
          case 'quorum.run.queued':
            next.runId = data.run_id;
            next.status = 'queued';
            break;

          case 'quorum.run.started':
            next.runId = data.run_id;
            next.status = 'running';
            break;

          case 'quorum.run.complete':
            next.status = 'complete';
            next.result = {
              runId: data.run_id,
              totalTokens: data.total_tokens,
              totalDurationMs: data.total_duration_ms,
              decomposeMs: data.decompose_ms,
              executeMs: data.execute_ms,
              synthesizeMs: data.synthesize_ms,
              finalOutput: data.final_output,
              workersSucceeded: data.workers_succeeded,
              workersFailed: data.workers_failed,
              workersRecovered: data.workers_recovered,
            };
            break;

          case 'quorum.run.failed':
            next.status = 'failed';
            next.error = data.error || 'Run failed';
            break;

          case 'quorum.phase.update':
            next.phase = {
              phase: data.phase,
              status: data.status,
              detail: data.detail,
              wallClockMs: data.wall_clock_ms,
              speedup: data.speedup,
              parallel: data.parallel,
              subtasks: data.subtasks,
            };
            if (data.subtasks) {
              next.subtasks = data.subtasks;
            }
            break;

          case 'quorum.agent.created':
            next.agents = {
              ...prev.agents,
              [data.agent_id]: {
                agentId: data.agent_id,
                role: data.role || 'worker',
                subtask: data.subtask,
                model: data.model,
                status: 'created',
              },
            };
            break;

          case 'quorum.agent.thinking':
            if (prev.agents[data.agent_id]) {
              next.agents = {
                ...prev.agents,
                [data.agent_id]: {
                  ...prev.agents[data.agent_id],
                  status: 'thinking',
                },
              };
            }
            break;

          case 'quorum.agent.output':
            if (prev.agents[data.agent_id]) {
              next.agents = {
                ...prev.agents,
                [data.agent_id]: {
                  ...prev.agents[data.agent_id],
                  status: 'success',
                  tokensUsed: data.tokens_used,
                  durationMs: data.duration_ms,
                  contentPreview: data.content_preview,
                  attempt: data.attempt,
                },
              };
            }
            break;

          case 'quorum.agent.failed':
            next.agents = {
              ...prev.agents,
              [data.agent_id]: {
                ...(prev.agents[data.agent_id] || { agentId: data.agent_id, role: 'worker' as const }),
                status: 'failed',
                error: data.error,
                attempt: data.attempts,
              },
            };
            break;

          case 'quorum.agent.recovered':
            next.agents = {
              ...prev.agents,
              [data.agent_id]: {
                agentId: data.agent_id,
                role: 'worker',
                status: 'recovered',
                tokensUsed: data.tokens_used,
                durationMs: data.duration_ms,
                contentPreview: data.content_preview,
                originalAgent: data.original_agent,
              },
            };
            break;

          case 'quorum.metrics.snapshot':
            next.metrics = {
              activeRuns: data.active_runs,
              queuedRuns: data.queued_runs,
              maxConcurrent: data.max_concurrent,
              cpuUsage: data.cpu_usage,
              memoryUsageMb: data.memory_usage_mb,
              tokensPerSec: data.tokens_per_sec,
              totalRunsCompleted: data.total_runs_completed,
              totalTokensUsed: data.total_tokens_used,
              avgRunDurationMs: data.avg_run_duration_ms,
            };
            break;

          case 'quorum.error':
            next.error = data.error || 'Unknown error';
            break;
        }

        return next;
      });
    } catch {
      // Ignore malformed messages
    }
  }, []);

  const submit = useCallback((request: QuorumSubmitRequest) => {
    // Reset state
    setState({
      runId: null,
      status: 'connecting',
      phase: null,
      agents: {},
      metrics: null,
      result: null,
      error: null,
      events: [],
      subtasks: [],
    });

    // Close any existing connection
    if (wsRef.current) {
      wsRef.current.close();
    }

    const ws = new WebSocket(ENGINE_WS_URL);
    wsRef.current = ws;

    ws.onopen = () => {
      // Subscribe to all quorum events first
      ws.send(JSON.stringify({ event: 'quorum.subscribe.all', data: {} }));

      // Submit the run
      ws.send(JSON.stringify({
        event: 'quorum.run.submit',
        data: request,
        timestamp: new Date().toISOString(),
      }));
    };

    ws.onmessage = (e) => handleEvent(e.data);

    ws.onerror = () => {
      setState(prev => ({ ...prev, status: 'failed', error: 'WebSocket connection error' }));
    };

    ws.onclose = () => {
      wsRef.current = null;
    };
  }, [handleEvent]);

  const subscribeToRun = useCallback((runId: string) => {
    // Reset state
    setState({
      runId,
      status: 'connecting',
      phase: null,
      agents: {},
      metrics: null,
      result: null,
      error: null,
      events: [],
      subtasks: [],
    });

    if (wsRef.current) {
      wsRef.current.close();
    }

    const ws = new WebSocket(ENGINE_WS_URL);
    wsRef.current = ws;

    ws.onopen = () => {
      ws.send(JSON.stringify({
        event: 'quorum.subscribe',
        data: { run_id: runId },
      }));
      setState(prev => ({ ...prev, status: 'running' }));
    };

    ws.onmessage = (e) => handleEvent(e.data);

    ws.onerror = () => {
      setState(prev => ({ ...prev, status: 'failed', error: 'WebSocket connection error' }));
    };

    ws.onclose = () => {
      wsRef.current = null;
    };
  }, [handleEvent]);

  const cancel = useCallback(() => {
    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN && state.runId) {
      ws.send(JSON.stringify({
        event: 'quorum.run.cancel',
        data: { run_id: state.runId },
      }));
    }
  }, [state.runId]);

  const disconnect = useCallback(() => {
    if (wsRef.current) {
      wsRef.current.close();
      wsRef.current = null;
    }
    setState(prev => ({ ...prev, status: 'idle' }));
  }, []);

  // Send raw quorum event
  const sendEvent = useCallback((event: string, data: Record<string, unknown> = {}) => {
    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ event, data, timestamp: new Date().toISOString() }));
    }
  }, []);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (wsRef.current) {
        wsRef.current.close();
      }
    };
  }, []);

  return {
    ...state,
    submit,
    subscribeToRun,
    cancel,
    disconnect,
    sendEvent,
  };
}
