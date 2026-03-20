import { NextRequest } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* POST /api/mcp/stream — SSE proxy for streaming MCP generation */
export async function POST(req: NextRequest) {
  const body = await req.json();

  const engineRes = await fetch(`${ENGINE_URL}/api/mcp/generate/stream`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      prompt: body.prompt,
      description: body.description || '',
      language: body.language || 'python',
      model: body.model || 'llama3.1:8b',
      source: body.source || 'ollama',
      provider_id: body.provider_id || '',
      system_prompt: body.system_prompt || '',
      prompt_type: body.prompt_type || 'mcp',
    }),
  });

  if (!engineRes.ok || !engineRes.body) {
    return new Response(JSON.stringify({ error: 'Stream failed' }), {
      status: 502,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  // Pipe the SSE stream from engine to browser
  return new Response(engineRes.body, {
    headers: {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache',
      'Connection': 'keep-alive',
      'X-Accel-Buffering': 'no',
    },
  });
}
