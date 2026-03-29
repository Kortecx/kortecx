import { NextRequest, NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* GET /api/experts/versions?expertId={id}&filename={name} — list versions of a file */
export async function GET(req: NextRequest) {
  const expertId = req.nextUrl.searchParams.get('expertId');
  const filename = req.nextUrl.searchParams.get('filename');

  if (!expertId || !filename) {
    return NextResponse.json({ error: 'expertId and filename required' }, { status: 400 });
  }

  try {
    const res = await fetch(
      `${ENGINE_URL}/api/agents/engine/${expertId}/versions/${encodeURIComponent(filename)}`,
    );
    const data = await res.json();
    return NextResponse.json(data);
  } catch (err) {
    console.error('[experts/versions GET]', err);
    return NextResponse.json({ versions: [], total: 0 });
  }
}

/* POST /api/experts/versions — restore a file to a previous version */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { expertId, version } = body;

    if (!expertId || !version) {
      return NextResponse.json({ error: 'expertId and version required' }, { status: 400 });
    }

    const res = await fetch(`${ENGINE_URL}/api/agents/engine/${expertId}/restore`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ version }),
    });

    const data = await res.json();
    return NextResponse.json(data, { status: res.status });
  } catch (err) {
    console.error('[experts/versions POST]', err);
    return NextResponse.json({ error: 'Failed to restore version' }, { status: 500 });
  }
}

export const dynamic = 'force-dynamic';
