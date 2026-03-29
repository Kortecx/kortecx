import { NextRequest, NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* POST /api/experts/graph/embed-marketplace — Bulk-embed marketplace templates into Qdrant */
export async function POST(req: NextRequest) {
  try {
    const { experts } = await req.json();
    if (!Array.isArray(experts) || experts.length === 0) {
      return NextResponse.json({ error: 'experts array required' }, { status: 400 });
    }
    const resp = await fetch(`${ENGINE_URL}/api/prism/engine/embed/bulk`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ experts, source: 'marketplace' }),
    });
    const data = await resp.json();
    return NextResponse.json(data);
  } catch (err) {
    console.error('[experts/graph/embed-marketplace POST]', err);
    return NextResponse.json({ error: 'Failed to embed marketplace experts' }, { status: 500 });
  }
}
