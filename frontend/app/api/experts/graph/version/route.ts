import { NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* GET /api/experts/graph/version — Lightweight Qdrant version check */
export async function GET() {
  try {
    const resp = await fetch(
      `${ENGINE_URL}/api/prism/engine/graph/version`,
      { cache: 'no-store' },
    );
    if (!resp.ok) return NextResponse.json({ count: 0, version: '0-0' });
    const data = await resp.json();
    return NextResponse.json(data);
  } catch {
    return NextResponse.json({ count: 0, version: '0-0' });
  }
}
