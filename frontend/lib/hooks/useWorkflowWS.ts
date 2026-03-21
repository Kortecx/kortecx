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

interface WorkflowLiveState {
  runId: string | null;
  status: 'idle' | 'connecting' | 'running' | 'completed' | 'failed';
  agents: Record<string, AgentLiveState>;
  sharedMemory: SharedMemory | null;
  events: WorkflowExecutionEvent[];
  output: string | null;
  error: string | null;
}

const ENGINE_WS_URL = process.env.NEXT_PUBLIC_ENGINE_WS_URL || 'ws://localhost:8000/ws';

export function useWorkflowWS() {
  const wsRef = useRef<WebSocket | null>(null);
  const [state, setState] = useState<WorkflowLiveState>({
    runId: null,
    status: 'idle',
    agents: {},
    sharedMemory: null,
    events: [],
    output: null,
    error: null,
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
            break;

          case 'workflow.failed':
            next.status = 'failed';
            next.error = data.error || 'Workflow failed';
            break;
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
      status: 'connecting',
      agents: {},
      sharedMemory: null,
      events: [],
      output: null,
      error: null,
    });

    const ws = new WebSocket(ENGINE_WS_URL);
    wsRef.current = ws;

    ws.onopen = () => {
      // Subscribe to workflow channel
      ws.send(JSON.stringify({
        event: 'subscribe',
        channel: `workflow.${runId}`,
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

  const executeViaWS = useCallback((request: Record<string, unknown>) => {
    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({
        event: 'workflow.execute',
        data: request,
      }));
    }
  }, []);

  const disconnect = useCallback(() => {
    if (wsRef.current) {
      wsRef.current.close();
      wsRef.current = null;
    }
    setState(prev => ({ ...prev, status: 'idle' }));
  }, []);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (wsRef.current) {
        wsRef.current.close();
      }
    };
  }, []);

  return { ...state, connect, executeViaWS, disconnect };
}
