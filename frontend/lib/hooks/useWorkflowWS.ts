'use client';

import { useState, useEffect, useCallback, useRef } from 'react';
import type { WorkflowExecutionEvent, WorkflowExecutionEventType, SharedMemory } from '@/lib/types';

interface AgentLiveState {
  agentId: string;
  stepId: string;
  status: 'idle' | 'queued' | 'spawned' | 'thinking' | 'waiting' | 'completed' | 'failed';
  taskDescription?: string;
  modelSource?: string;
  stepName?: string;
  model?: string;
  engine?: string;
  output?: string;
  tokensUsed?: number;
  durationMs?: number;
  cpuPercent?: number;
  gpuPercent?: number;
  memoryMb?: number;
  startedAt?: string;
  completedAt?: string;
  error?: string;
}

export interface LiveMetrics {
  cpuPercent: number;
  gpuPercent: number;
  memoryMb: number;
  totalTokensUsed: number;
  elapsedMs: number;
}

interface WorkflowLiveState {
  runId: string | null;
  workflowId: string | null;
  status: 'idle' | 'connecting' | 'running' | 'completed' | 'failed' | 'cancelled';
  agents: Record<string, AgentLiveState>;
  sharedMemory: SharedMemory | null;
  events: WorkflowExecutionEvent[];
  output: string | null;
  error: string | null;
  liveMetrics: LiveMetrics | null;
  /** Per-run live metrics when subscribed to multiple runs */
  runMetrics: Record<string, LiveMetrics>;
}

const ENGINE_WS_URL = process.env.NEXT_PUBLIC_ENGINE_WS_URL || 'ws://localhost:8000/ws';

export function useWorkflowWS() {
  const wsRef = useRef<WebSocket | null>(null);
  const subscribedChannelsRef = useRef<Set<string>>(new Set());
  const workflowIdRef = useRef<string | null>(null);
  const [state, setState] = useState<WorkflowLiveState>({
    runId: null,
    workflowId: null,
    status: 'idle',
    agents: {},
    sharedMemory: null,
    events: [],
    output: null,
    error: null,
    liveMetrics: null,
    runMetrics: {},
  });

  const handleEvent = useCallback((raw: string) => {
    try {
      const msg = JSON.parse(raw);
      const event = msg.event as WorkflowExecutionEventType;
      const data = msg.data || {};

      const execEvent: WorkflowExecutionEvent = {
        runId: data.runId || '',
        event,
        agentId: data.agentId,
        stepId: data.stepId,
        data,
        timestamp: msg.timestamp || new Date().toISOString(),
      };

      setState(prev => {
        const next = { ...prev, events: [...prev.events, execEvent] };

        switch (event) {
          case 'agent.spawned':
            next.agents = {
              ...prev.agents,
              [data.agentId]: {
                agentId: data.agentId,
                stepId: data.stepId,
                status: 'spawned',
                taskDescription: data.taskDescription,
                modelSource: data.modelSource,
                stepName: data.stepName,
                model: data.model,
                engine: data.engine,
                startedAt: msg.timestamp || new Date().toISOString(),
              },
            };
            break;

          case 'agent.thinking':
            if (prev.agents[data.agentId]) {
              next.agents = {
                ...prev.agents,
                [data.agentId]: {
                  ...prev.agents[data.agentId],
                  status: 'thinking',
                  startedAt: data.startedAt || prev.agents[data.agentId].startedAt,
                },
              };
            }
            break;

          case 'agent.memory.update':
            if (data.sharedMemory) {
              next.sharedMemory = data.sharedMemory as SharedMemory;
            }
            break;

          case 'agent.step.complete':
            if (prev.agents[data.agentId]) {
              next.agents = {
                ...prev.agents,
                [data.agentId]: {
                  ...prev.agents[data.agentId],
                  status: 'completed',
                  output: data.output,
                  tokensUsed: data.tokensUsed,
                  durationMs: data.durationMs,
                  cpuPercent: data.cpuPercent,
                  gpuPercent: data.gpuPercent,
                  memoryMb: data.memoryMb,
                  completedAt: new Date().toISOString(),
                },
              };
            }
            // Persist step execution metrics to DB
            fetch('/api/workflows/executions', {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({
                runId: data.runId || execEvent.runId,
                stepId: data.stepId,
                agentId: data.agentId,
                status: 'completed',
                tokensUsed: data.tokensUsed,
                durationMs: data.durationMs,
                cpuPercent: data.cpuPercent,
                gpuPercent: data.gpuPercent,
                memoryMb: data.memoryMb,
                model: data.model,
                engine: data.engine,
                responsePreview: typeof data.output === 'string' ? data.output.slice(0, 500) : '',
                completedAt: new Date().toISOString(),
              }),
            }).catch(() => {});
            break;

          case 'agent.step.failed':
            if (prev.agents[data.agentId]) {
              next.agents = {
                ...prev.agents,
                [data.agentId]: {
                  ...prev.agents[data.agentId],
                  status: 'failed',
                  error: data.error,
                },
              };
            }
            // Persist failed step execution
            fetch('/api/workflows/executions', {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({
                runId: data.runId || execEvent.runId,
                stepId: data.stepId,
                agentId: data.agentId,
                status: 'failed',
                errorMessage: data.error,
                completedAt: new Date().toISOString(),
              }),
            }).catch(() => {});
            break;

          case 'workflow.complete':
            next.status = 'completed';
            next.output = data.output || null;
            if (data.sharedMemory) next.sharedMemory = data.sharedMemory;
            next.liveMetrics = null;
            // Sync terminal status to workflows table
            if (workflowIdRef.current) {
              fetch('/api/workflows', {
                method: 'PATCH',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ id: workflowIdRef.current, status: 'completed', updatedAt: new Date().toISOString() }),
              }).catch(() => {});
            }
            break;

          case 'workflow.failed':
            next.status = 'failed';
            next.error = data.error || 'Workflow failed';
            next.liveMetrics = null;
            // Sync terminal status to workflows table
            if (workflowIdRef.current) {
              fetch('/api/workflows', {
                method: 'PATCH',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ id: workflowIdRef.current, status: 'failed', updatedAt: new Date().toISOString() }),
              }).catch(() => {});
            }
            break;

          case 'workflow.cancelled' as WorkflowExecutionEventType:
            next.status = 'cancelled';
            next.error = data.message || 'Workflow cancelled';
            next.liveMetrics = null;
            // Sync terminal status to workflows table
            if (workflowIdRef.current) {
              fetch('/api/workflows', {
                method: 'PATCH',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ id: workflowIdRef.current, status: 'cancelled', updatedAt: new Date().toISOString() }),
              }).catch(() => {});
            }
            break;

          case 'run.metrics.update' as WorkflowExecutionEventType: {
            const metrics: LiveMetrics = {
              cpuPercent: data.cpuPercent ?? 0,
              gpuPercent: data.gpuPercent ?? 0,
              memoryMb: data.memoryMb ?? 0,
              totalTokensUsed: data.tokensUsed ?? 0,
              elapsedMs: data.elapsedMs ?? 0,
            };
            next.liveMetrics = metrics;
            // Also track per-run metrics for multi-run support
            if (data.runId) {
              next.runMetrics = {
                ...prev.runMetrics,
                [data.runId]: metrics,
              };
            }
            // Update agent-level metrics
            if (data.agentId && prev.agents[data.agentId]) {
              next.agents = {
                ...prev.agents,
                [data.agentId]: {
                  ...prev.agents[data.agentId],
                  cpuPercent: data.cpuPercent,
                  gpuPercent: data.gpuPercent,
                  memoryMb: data.memoryMb,
                  tokensUsed: data.tokensUsed ?? prev.agents[data.agentId].tokensUsed,
                },
              };
            }
            break;
          }
        }

        return next;
      });
    } catch {
      // Ignore malformed messages
    }
  }, []);

  const connect = useCallback((runId: string) => {
    // Close any existing connection
    if (wsRef.current) {
      wsRef.current.close();
    }

    setState({
      runId,
      workflowId: workflowIdRef.current,
      status: 'connecting',
      agents: {},
      sharedMemory: null,
      events: [],
      output: null,
      error: null,
      liveMetrics: null,
      runMetrics: {},
    });

    const ws = new WebSocket(ENGINE_WS_URL);
    wsRef.current = ws;

    ws.onopen = () => {
      // Subscribe to workflow channel
      ws.send(JSON.stringify({
        event: 'subscribe',
        channel: `workflow.${runId}`,
      }));
      subscribedChannelsRef.current.add(`workflow.${runId}`);
      setState(prev => ({ ...prev, status: 'running' }));
    };

    ws.onmessage = (e) => handleEvent(e.data);

    ws.onerror = () => {
      setState(prev => ({ ...prev, status: 'failed', error: 'WebSocket connection error' }));
    };

    ws.onclose = () => {
      wsRef.current = null;
      subscribedChannelsRef.current.clear();
    };
  }, [handleEvent]);

  /** Subscribe to multiple run channels on a single WebSocket connection */
  const subscribeToRuns = useCallback((runIds: string[]) => {
    // Ensure we have a connection
    if (!wsRef.current || wsRef.current.readyState !== WebSocket.OPEN) {
      // Create new connection
      const ws = new WebSocket(ENGINE_WS_URL);
      wsRef.current = ws;

      ws.onopen = () => {
        for (const runId of runIds) {
          const channel = `workflow.${runId}`;
          if (!subscribedChannelsRef.current.has(channel)) {
            ws.send(JSON.stringify({ event: 'subscribe', channel }));
            subscribedChannelsRef.current.add(channel);
          }
        }
        setState(prev => ({ ...prev, status: 'running' }));
      };

      ws.onmessage = (e) => handleEvent(e.data);
      ws.onerror = () => {};
      ws.onclose = () => {
        wsRef.current = null;
        subscribedChannelsRef.current.clear();
      };
    } else {
      // Subscribe to new channels on existing connection
      for (const runId of runIds) {
        const channel = `workflow.${runId}`;
        if (!subscribedChannelsRef.current.has(channel)) {
          wsRef.current.send(JSON.stringify({ event: 'subscribe', channel }));
          subscribedChannelsRef.current.add(channel);
        }
      }
    }
  }, [handleEvent]);

  /**
   * Submit a workflow for execution via WebSocket.
   * Establishes connection if needed, subscribes to the run channel,
   * and sends the workflow.execute event.
   */
  const submitWorkflow = useCallback((runId: string, request: Record<string, unknown>) => {
    // Track workflowId for status sync on terminal events
    if (request.workflowId) {
      workflowIdRef.current = request.workflowId as string;
    }

    const sendExecute = (ws: WebSocket) => {
      // Subscribe to the run channel for real-time updates
      const channel = `workflow.${runId}`;
      if (!subscribedChannelsRef.current.has(channel)) {
        ws.send(JSON.stringify({ event: 'subscribe', channel }));
        subscribedChannelsRef.current.add(channel);
      }
      // Send execute command with runId
      ws.send(JSON.stringify({
        event: 'workflow.execute',
        data: { ...request, runId },
      }));
    };

    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN) {
      setState(prev => ({ ...prev, runId, status: 'running' }));
      sendExecute(ws);
      return;
    }

    // Establish new connection
    setState({
      runId,
      workflowId: (request.workflowId as string) || null,
      status: 'connecting',
      agents: {},
      sharedMemory: null,
      events: [],
      output: null,
      error: null,
      liveMetrics: null,
      runMetrics: {},
    });

    const newWs = new WebSocket(ENGINE_WS_URL);
    wsRef.current = newWs;

    newWs.onopen = () => {
      setState(prev => ({ ...prev, status: 'running' }));
      sendExecute(newWs);
    };

    newWs.onmessage = (e) => handleEvent(e.data);
    newWs.onerror = () => {
      setState(prev => ({ ...prev, status: 'failed', error: 'WebSocket connection error' }));
    };
    newWs.onclose = () => {
      wsRef.current = null;
      subscribedChannelsRef.current.clear();
    };
  }, [handleEvent]);

  const disconnect = useCallback(() => {
    if (wsRef.current) {
      wsRef.current.close();
      wsRef.current = null;
    }
    subscribedChannelsRef.current.clear();
    setState(prev => ({ ...prev, status: 'idle', liveMetrics: null, runMetrics: {} }));
  }, []);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (wsRef.current) {
        wsRef.current.close();
      }
    };
  }, []);

  return { ...state, connect, subscribeToRuns, submitWorkflow, disconnect };
}
