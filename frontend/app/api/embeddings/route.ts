import { NextRequest, NextResponse } from 'next/server';

const ENGINE = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

export async function GET(req: NextRequest) {
  const { searchParams } = new URL(req.url);
  const action = searchParams.get('action') || 'collections';

  try {
    if (action === 'collections') {
      const res = await fetch(`${ENGINE}/api/embeddings/collections`);
      const data = await res.json();
      return NextResponse.json(data);
    }
    if (action === 'search') {
      const query = searchParams.get('query') || '';
      const collection = searchParams.get('collection') || '';
      const limit = searchParams.get('limit') || '10';
      const res = await fetch(`${ENGINE}/api/embeddings/search`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ query, collection, limit: parseInt(limit) }),
      });
      const data = await res.json();
      return NextResponse.json(data);
    }
    return NextResponse.json({ error: 'Unknown action' }, { status: 400 });
  } catch (e: unknown) {
    return NextResponse.json({ error: e instanceof Error ? e.message : 'Engine unreachable' }, { status: 502 });
  }
}

export async function POST(req: NextRequest) {
  const body = await req.json();
  const action = body.action || 'embed';

  try {
    if (action === 'embed') {
      const res = await fetch(`${ENGINE}/api/embeddings/embed`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ texts: body.texts, collection: body.collection }),
      });
      return NextResponse.json(await res.json());
    }
    if (action === 'upsert') {
      const res = await fetch(`${ENGINE}/api/embeddings/upsert`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      return NextResponse.json(await res.json());
    }
    return NextResponse.json({ error: 'Unknown action' }, { status: 400 });
  } catch (e: unknown) {
    return NextResponse.json({ error: e instanceof Error ? e.message : 'Engine unreachable' }, { status: 502 });
  }
}
