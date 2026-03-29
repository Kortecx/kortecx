import { NextRequest } from 'next/server';

const OLLAMA_URL = process.env.OLLAMA_URL || 'http://localhost:11434';
const DEFAULT_MODEL = process.env.DEFAULT_LOCAL_MODEL || 'llama3.1:8b';

/**
 * POST /api/quick-check/stream
 *
 * Streams Ollama generate tokens back to the frontend as NDJSON lines.
 * Each line: { "token": "..." }            — a token chunk
 * Final line: { "done": true, "model": "...", "tokensUsed": N, "durationMs": N }
 *
 * This route talks directly to Ollama from the Next.js server,
 * so the browser never needs to reach Ollama or the engine.
 */
export async function POST(req: NextRequest) {
  let body: { prompt?: string; model?: string; checkId?: string };
  try {
    body = await req.json();
  } catch {
    return new Response(JSON.stringify({ error: 'Invalid JSON body' }), {
      status: 400,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  const prompt = body.prompt?.trim();
  if (!prompt) {
    return new Response(JSON.stringify({ error: 'prompt is required' }), {
      status: 400,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  const model = body.model || DEFAULT_MODEL;

  // Build system prompt with minimal platform context
  const system =
    'You are the Kortecx platform assistant. ' +
    'Help the user understand their platform, data, workflows, and AI agents. ' +
    'Answer concisely and accurately.';

  const ollamaPayload = {
    model,
    prompt,
    system,
    stream: true,
    options: { temperature: 0.7, num_predict: 4096 },
  };

  // Call Ollama streaming API
  let ollamaRes: Response;
  try {
    ollamaRes = await fetch(`${OLLAMA_URL}/api/generate`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(ollamaPayload),
    });
  } catch (err) {
    const msg =
      err instanceof TypeError && String(err).includes('fetch')
        ? `Cannot connect to Ollama at ${OLLAMA_URL}. Ensure Ollama is running.`
        : `Ollama request failed: ${String(err)}`;
    return new Response(JSON.stringify({ error: msg }), {
      status: 502,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  if (!ollamaRes.ok) {
    const text = await ollamaRes.text().catch(() => '');
    const msg = text.includes('not found')
      ? `Model '${model}' not found. Pull it first with: ollama pull ${model}`
      : `Ollama returned ${ollamaRes.status}: ${text.slice(0, 200)}`;
    return new Response(JSON.stringify({ error: msg }), {
      status: ollamaRes.status,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  if (!ollamaRes.body) {
    return new Response(JSON.stringify({ error: 'Ollama returned no body' }), {
      status: 502,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  // Pipe Ollama NDJSON → our NDJSON, transforming to our format
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
          // If we never got a "done" chunk from Ollama, emit one now
          const final = JSON.stringify({
            done: true,
            model,
            tokensUsed,
            durationMs: Date.now() - startMs,
          });
          controller.enqueue(encoder.encode(final + '\n'));
          controller.close();
          return;
        }

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || ''; // keep incomplete last line in buffer

        for (const line of lines) {
          if (!line.trim()) continue;
          try {
            const chunk = JSON.parse(line);

            if (chunk.done) {
              // Final chunk from Ollama — emit our done event
              const evalCount = (chunk.eval_count || 0) + (chunk.prompt_eval_count || 0);
              const final = JSON.stringify({
                done: true,
                model,
                tokensUsed: evalCount || tokensUsed,
                durationMs: chunk.total_duration
                  ? Math.round(chunk.total_duration / 1_000_000)
                  : Date.now() - startMs,
              });
              controller.enqueue(encoder.encode(final + '\n'));
              controller.close();
              return;
            }

            const token = chunk.response || '';
            if (token) {
              tokensUsed++;
              controller.enqueue(
                encoder.encode(JSON.stringify({ token }) + '\n'),
              );
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
