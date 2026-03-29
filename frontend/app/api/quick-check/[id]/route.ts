import { NextRequest, NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* ── GET — proxy quick-check polling to the engine ── */
export async function GET(
  _req: NextRequest,
  { params }: { params: Promise<{ id: string }> },
) {
  try {
    const { id } = await params;

    const res = await fetch(`${ENGINE_URL}/api/quick-check/${id}`);
    const data = await res.json();
    return NextResponse.json(data, { status: res.status });
  } catch (err) {
    console.error('GET /api/quick-check/[id] proxy error:', err);
    return NextResponse.json(
      { error: 'Could not reach the engine. Ensure it is running.' },
      { status: 502 },
    );
  }
}
