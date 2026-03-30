import { NextRequest, NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/** Retry fetch on connection errors (engine may still be booting). */
async function fetchWithRetry(url: string, init?: RequestInit, retries = 2, delayMs = 2000): Promise<Response> {
  for (let attempt = 0; attempt <= retries; attempt++) {
    try {
      return await fetch(url, init);
    } catch (err: unknown) {
      const isConnErr = err instanceof TypeError && (
        (err.cause as { code?: string })?.code === 'ECONNREFUSED' ||
        String(err.message).includes('fetch failed')
      );
      if (!isConnErr || attempt === retries) throw err;
      await new Promise(r => setTimeout(r, delayMs));
    }
  }
  throw new Error('unreachable');
}

/* POST /api/experts/graph/embed-marketplace — Bulk-embed marketplace templates into Qdrant */
export async function POST(req: NextRequest) {
  try {
    const { experts } = await req.json();
    if (!Array.isArray(experts) || experts.length === 0) {
      return NextResponse.json({ error: 'experts array required' }, { status: 400 });
    }
    const resp = await fetchWithRetry(
      `${ENGINE_URL}/api/agents/engine/embed/bulk`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ experts, source: 'marketplace' }),
      },
    );
    const data = await resp.json();
    return NextResponse.json(data);
  } catch {
    return NextResponse.json({ error: 'Failed to embed marketplace experts' }, { status: 500 });
  }
}
