import { NextRequest, NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* GET /api/experts/files?expertId={id} — list files for an expert */
export async function GET(req: NextRequest) {
  const expertId = req.nextUrl.searchParams.get('expertId');
  if (!expertId) {
    return NextResponse.json({ error: 'expertId required' }, { status: 400 });
  }

  try {
    const res = await fetch(`${ENGINE_URL}/api/prism/engine/${expertId}/files`);
    const data = await res.json();
    return NextResponse.json(data);
  } catch (err) {
    console.error('[experts/files GET]', err);
    return NextResponse.json({ files: [] });
  }
}

/* POST /api/experts/files — update a file for an expert */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { expertId, filename, content } = body;

    if (!expertId || !filename || content === undefined) {
      return NextResponse.json(
        { error: 'expertId, filename, and content are required' },
        { status: 400 },
      );
    }

    const res = await fetch(`${ENGINE_URL}/api/prism/engine/${expertId}/update`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ filename, content }),
    });

    const data = await res.json();
    return NextResponse.json(data, { status: res.status });
  } catch (err) {
    console.error('[experts/files POST]', err);
    return NextResponse.json({ error: 'Failed to update file' }, { status: 500 });
  }
}

export const dynamic = 'force-dynamic';
