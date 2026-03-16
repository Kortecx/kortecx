import { NextRequest, NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* POST /api/synthesis/models/delete — delete a local model */
export async function POST(req: NextRequest) {
  try {
    const { source, model } = await req.json();
    if (!source || !model) {
      return NextResponse.json({ error: 'source and model are required' }, { status: 400 });
    }

    if (source === 'ollama') {
      // Ollama delete via engine orchestrator endpoint
      const res = await fetch(`${ENGINE_URL}/api/orchestrator/models/delete`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ engine: 'ollama', model }),
      });
      if (!res.ok) {
        const err = await res.text();
        return NextResponse.json({ error: err }, { status: res.status });
      }
      return NextResponse.json({ deleted: true, model, source });
    }

    return NextResponse.json({ error: `Delete not supported for source: ${source}` }, { status: 400 });
  } catch (err) {
    console.error('[models/delete POST]', err);
    return NextResponse.json({ error: 'Delete failed' }, { status: 500 });
  }
}
