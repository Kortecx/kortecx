import { NextRequest } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';
const OLLAMA_URL = process.env.OLLAMA_URL || 'http://localhost:11434';
const DEFAULT_MODEL = process.env.DEFAULT_LOCAL_MODEL || 'llama3.1:8b';

const SYSTEM_PROMPT =
  'You are the Kortecx platform assistant. ' +
  'Help the user understand their platform, data, workflows, and AI agents. ' +
  'Answer concisely and accurately.';

/**
 * POST /api/quick-check/stream
 *
 * Streams inference tokens back to the frontend as NDJSON lines.
 * Each line: { "token": "..." }            — a token chunk
 * Final line: { "done": true, "model": "...", "tokensUsed": N, "durationMs": N }
 *
 * Strategy: try engine first (sanctioned path with retry/pool tracking),
 * fall back to direct Ollama if engine is unreachable.
 */
export async function POST(req: NextRequest) {
  let body: { prompt?: string; model?: string; checkId?: string };
  try {
    body = await req.json();
  } catch {
    return jsonError('Invalid JSON body', 400);
  }

  const prompt = body.prompt?.trim();
  if (!prompt) {
    return jsonError('prompt is required', 400);
  }

  const model = body.model || DEFAULT_MODEL;

  // ── 1. Try engine (sanctioned path) ────────────────────────────────────
  try {
    const engineRes = await fetch(`${ENGINE_URL}/api/orchestrator/inference/generate`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        engine: 'ollama',
        model,
        prompt,
        system: SYSTEM_PROMPT,
        temperature: 0.7,
        maxTokens: 4096,
      }),
    });

    if (engineRes.ok) {
      const data = await engineRes.json();
      // Engine returns the full response at once — emit as NDJSON stream
      const encoder = new TextEncoder();
      const text = data.text || data.response || '';
      const tokensUsed = data.tokens_used ?? data.tokensUsed ?? 0;
      const durationMs = Math.round(data.duration_ms ?? data.durationMs ?? 0);
      const cpuPercent = await fetchCpuPercent();

      const ndjson =
        JSON.stringify({ token: text }) + '\n' +
        JSON.stringify({ done: true, model, tokensUsed, durationMs, cpuPercent }) + '\n';

      return new Response(encoder.encode(ndjson), {
        headers: {
          'Content-Type': 'application/x-ndjson',
          'Cache-Control': 'no-cache',
        },
      });
    }
    // Engine returned non-OK — fall through to Ollama fallback
  } catch {
    // Engine unreachable — fall through to Ollama fallback
  }

  // ── 2. Fallback: direct Ollama streaming ───────────────────────────────
  return streamFromOllama(prompt, model);
}

/* ── Ollama streaming fallback ────────────────────────────────────────── */

async function streamFromOllama(prompt: string, model: string): Promise<Response> {
  let ollamaRes: Response;
  try {
    ollamaRes = await fetch(`${OLLAMA_URL}/api/generate`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        model,
        prompt,
        system: SYSTEM_PROMPT,
        stream: true,
        options: { temperature: 0.7, num_predict: 4096 },
      }),
    });
  } catch (err) {
    const msg =
      err instanceof TypeError && String(err).includes('fetch')
        ? `Cannot connect to Ollama at ${OLLAMA_URL}. Ensure Ollama is running.`
        : `Ollama request failed: ${String(err)}`;
    return jsonError(msg, 502);
  }

  if (!ollamaRes.ok) {
    const text = await ollamaRes.text().catch(() => '');
    const msg = text.includes('not found')
      ? `Model '${model}' not found. Pull it first with: ollama pull ${model}`
      : `Ollama returned ${ollamaRes.status}: ${text.slice(0, 200)}`;
    return jsonError(msg, ollamaRes.status);
  }

  if (!ollamaRes.body) {
    return jsonError('Ollama returned no body', 502);
  }

  const startMs = Date.now();
  let tokensUsed = 0;
  const reader = ollamaRes.body.getReader();
  const decoder = new TextDecoder();
  const encoder = new TextEncoder();
  let buffer = '';

  const stream = new ReadableStream({
    async pull(controller) {
      try {
        const { done, value } = await reader.read();

        if (done) {
          const cpuPercent = await fetchCpuPercent();
          controller.enqueue(encoder.encode(
            JSON.stringify({ done: true, model, tokensUsed, durationMs: Date.now() - startMs, cpuPercent }) + '\n',
          ));
          controller.close();
          return;
        }

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';

        for (const line of lines) {
          if (!line.trim()) continue;
          try {
            const chunk = JSON.parse(line);

            if (chunk.done) {
              const evalCount = (chunk.eval_count || 0) + (chunk.prompt_eval_count || 0);
              const cpuPercent = await fetchCpuPercent();
              controller.enqueue(encoder.encode(
                JSON.stringify({
                  done: true,
                  model,
                  tokensUsed: evalCount || tokensUsed,
                  durationMs: chunk.total_duration
                    ? Math.round(chunk.total_duration / 1_000_000)
                    : Date.now() - startMs,
                  cpuPercent,
                }) + '\n',
              ));
              controller.close();
              return;
            }

            const token = chunk.response || '';
            if (token) {
              tokensUsed++;
              controller.enqueue(encoder.encode(JSON.stringify({ token }) + '\n'));
            }
          } catch {
            // skip unparseable lines
          }
        }
      } catch (err) {
        controller.error(err);
      }
    },
    cancel() {
      reader.cancel();
    },
  });

  return new Response(stream, {
    headers: {
      'Content-Type': 'application/x-ndjson',
      'Cache-Control': 'no-cache',
      'Transfer-Encoding': 'chunked',
    },
  });
}

/* ── Helpers ──────────────────────────────────────────────────────────── */

async function fetchCpuPercent(): Promise<number> {
  try {
    const res = await fetch(`${ENGINE_URL}/api/orchestrator/system/stats`);
    if (res.ok) {
      const data = await res.json();
      return data.cpu_percent ?? 0;
    }
  } catch { /* engine may be down */ }
  return 0;
}

function jsonError(message: string, status: number): Response {
  return new Response(JSON.stringify({ error: message }), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}
