'use client';

import { useState, useCallback, useRef, useEffect } from 'react';

const ENGINE_HTTP_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

function getEngineWsUrl(): string {
  const explicit = process.env.NEXT_PUBLIC_ENGINE_WS_URL;
  if (explicit) return explicit;
  const wsProto = ENGINE_HTTP_URL.startsWith('https') ? 'wss' : 'ws';
  return `${wsProto}://${ENGINE_HTTP_URL.replace(/^https?:\/\//, '')}/ws`;
}

type QCStatus = 'idle' | 'connecting' | 'streaming' | 'completed' | 'error';
type QCPhase = 'connecting' | 'gathering_context' | 'generating' | null;

interface QuickCheckState {
  checkId: string | null;
  status: QCStatus;
  phase: QCPhase;
  response: string;
  error: string | null;
  tokensUsed: number;
  durationMs: number;
  model: string | null;
  cpuPercent: number | null;
}

const INITIAL_STATE: QuickCheckState = {
  checkId: null,
  status: 'idle',
  phase: null,
  response: '',
  error: null,
  tokensUsed: 0,
  durationMs: 0,
  model: null,
  cpuPercent: null,
};

const MAX_WS_RETRIES = 2;
const RETRY_DELAY_MS = 1000;

export function useQuickCheckWS() {
  const wsRef = useRef<WebSocket | null>(null);
  const lastSubmitRef = useRef<{ checkId: string; prompt: string } | null>(null);
  const [state, setState] = useState<QuickCheckState>(INITIAL_STATE);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (wsRef.current) {
        try { wsRef.current.close(); } catch { /* ignore */ }
      }
    };
  }, []);

  // ── Streaming HTTP fallback (Next.js → Ollama direct) ─────────────────
  const submitViaStream = useCallback(async (checkId: string, prompt: string) => {
    setState(prev => ({ ...prev, status: 'connecting', phase: 'connecting' }));

    try {
      const res = await fetch('/api/quick-check/stream', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ checkId, prompt }),
      });

      if (!res.ok) {
        const data = await res.json().catch(() => ({ error: `Status ${res.status}` }));
        setState(prev => ({
          ...prev, status: 'error', phase: null,
          error: data.error || `Ollama returned status ${res.status}`,
        }));
        return;
      }

      if (!res.body) {
        setState(prev => ({
          ...prev, status: 'error', phase: null,
          error: 'No streaming response from server',
        }));
        return;
      }

      setState(prev => ({ ...prev, status: 'streaming', phase: 'generating' }));

      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';

        for (const line of lines) {
          if (!line.trim()) continue;
          try {
            const chunk = JSON.parse(line);

            if (chunk.done) {
              setState(prev => ({
                ...prev,
                status: 'completed',
                phase: null,
                tokensUsed: chunk.tokensUsed ?? prev.tokensUsed,
                durationMs: chunk.durationMs ?? 0,
                model: chunk.model ?? null,
                cpuPercent: chunk.cpuPercent ?? null,
              }));
              return;
            }

            if (chunk.token) {
              setState(prev => ({
                ...prev,
                status: 'streaming',
                phase: 'generating',
                response: prev.response + chunk.token,
                tokensUsed: prev.tokensUsed + 1,
              }));
            }
          } catch { /* skip unparseable lines */ }
        }
      }

      // Stream ended without a done event — mark completed
      setState(prev => {
        if (prev.status === 'streaming') {
          return { ...prev, status: 'completed', phase: null };
        }
        return prev;
      });

    } catch {
      setState(prev => ({
        ...prev, status: 'error', phase: null,
        error: 'Could not connect to Ollama. Ensure Ollama is running.',
      }));
    }
  }, []);

  // ── WebSocket submit with retry ────────────────────────────────────────
  const submit = useCallback((checkId: string, prompt: string) => {
    // Close any existing connection
    if (wsRef.current) {
      try { wsRef.current.close(); } catch { /* ignore */ }
    }

    lastSubmitRef.current = { checkId, prompt };

    setState({
      ...INITIAL_STATE,
      checkId,
      status: 'connecting',
      phase: 'connecting',
    });

    let retries = 0;

    function attemptWs() {
      const ws = new WebSocket(getEngineWsUrl());
      wsRef.current = ws;

      ws.onopen = () => {
        setState(prev => ({ ...prev, phase: 'gathering_context' }));
        ws.send(JSON.stringify({ event: 'subscribe', channel: `quick_check.${checkId}` }));
        ws.send(JSON.stringify({
          event: 'quick_check.submit',
          data: { checkId, prompt },
        }));
      };

      ws.onmessage = (evt) => {
        try {
          const msg = JSON.parse(evt.data);
          const event = msg.event as string;

          if (event === 'quick_check.accepted') {
            setState(prev => ({ ...prev, status: 'streaming', phase: 'generating' }));
          } else if (event === 'quick_check.token') {
            const token = msg.data?.token ?? '';
            setState(prev => ({
              ...prev,
              status: 'streaming',
              phase: 'generating',
              response: prev.response + token,
              tokensUsed: prev.tokensUsed + 1,
            }));
          } else if (event === 'quick_check.completed') {
            setState(prev => ({
              ...prev,
              status: 'completed',
              phase: null,
              response: msg.data?.response ?? prev.response,
              tokensUsed: msg.data?.tokensUsed ?? prev.tokensUsed,
              durationMs: msg.data?.durationMs ?? 0,
              model: msg.data?.model ?? null,
              cpuPercent: msg.data?.cpuPercent ?? null,
            }));
            ws.close();
          } else if (event === 'quick_check.error') {
            setState(prev => ({
              ...prev,
              status: 'error',
              phase: null,
              error: msg.data?.error ?? 'Unknown error',
            }));
            ws.close();
          }
        } catch { /* ignore parse errors */ }
      };

      ws.onerror = () => {
        if (retries < MAX_WS_RETRIES) {
          retries++;
          wsRef.current = null;
          setTimeout(attemptWs, RETRY_DELAY_MS);
        } else {
          // All WS retries exhausted — fall back to HTTP polling
          wsRef.current = null;
          submitViaStream(checkId, prompt);
        }
      };

      ws.onclose = () => {
        wsRef.current = null;
      };
    }

    attemptWs();
  }, [submitViaStream]);

  // ── Retry with last prompt ─────────────────────────────────────────────
  const retry = useCallback(() => {
    const last = lastSubmitRef.current;
    if (last) {
      submit(last.checkId, last.prompt);
    }
  }, [submit]);

  const reset = useCallback(() => {
    if (wsRef.current) {
      try { wsRef.current.close(); } catch { /* ignore */ }
      wsRef.current = null;
    }
    setState(INITIAL_STATE);
  }, []);

  return {
    submit,
    reset,
    retry,
    checkId: state.checkId,
    status: state.status,
    phase: state.phase,
    response: state.response,
    error: state.error,
    tokensUsed: state.tokensUsed,
    durationMs: state.durationMs,
    model: state.model,
    cpuPercent: state.cpuPercent,
  };
}
