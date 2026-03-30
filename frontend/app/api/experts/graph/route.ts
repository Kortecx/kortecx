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

/* GET /api/experts/graph — Fetch similarity edges from engine (Qdrant) */
export async function GET(req: NextRequest) {
  const { searchParams } = req.nextUrl;
  const threshold = searchParams.get('threshold') ?? '0.15';
  const limit = searchParams.get('limit') ?? '30';
  const source = searchParams.get('source') ?? '';

  try {
    const sourceParam = source ? `&source=${encodeURIComponent(source)}` : '';
    const resp = await fetchWithRetry(
      `${ENGINE_URL}/api/agents/engine/graph/edges?threshold=${threshold}&limit=${limit}${sourceParam}`,
      { cache: 'no-store' },
    );
    if (!resp.ok) {
      return NextResponse.json({ edges: [], total: 0 });
    }
    const data = await resp.json();
    return NextResponse.json(data);
  } catch {
    return NextResponse.json({ edges: [], total: 0 });
  }
}

/* POST /api/experts/graph — Create explicit edge between two agents */
export async function POST(req: NextRequest) {
  try {
    const { source, target } = await req.json();
    if (!source || !target) {
      return NextResponse.json({ error: 'source and target required' }, { status: 400 });
    }
    const resp = await fetch(
      `${ENGINE_URL}/api/agents/engine/${source}/attach`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ targetId: target }),
      },
    );
    const data = await resp.json();
    return NextResponse.json(data);
  } catch (err) {
    console.error('[experts/graph POST]', err);
    return NextResponse.json({ error: 'Failed to create edge' }, { status: 500 });
  }
}
