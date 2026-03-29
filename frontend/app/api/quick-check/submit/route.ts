import { NextRequest, NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* ── POST — proxy quick-check submission to the engine ── */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();

    const res = await fetch(`${ENGINE_URL}/api/quick-check/submit`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });

    const data = await res.json();
    return NextResponse.json(data, { status: res.status });
  } catch (err) {
    console.error('POST /api/quick-check/submit proxy error:', err);
    return NextResponse.json(
      { error: 'Could not reach the engine. Ensure it is running.' },
      { status: 502 },
    );
  }
}
