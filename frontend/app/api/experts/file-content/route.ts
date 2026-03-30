import { NextRequest, NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* GET /api/experts/file-content?expertId={id}&filename={name} — get file content */
export async function GET(req: NextRequest) {
  const expertId = req.nextUrl.searchParams.get('expertId');
  const filename = req.nextUrl.searchParams.get('filename');

  if (!expertId || !filename) {
    return NextResponse.json({ error: 'expertId and filename required' }, { status: 400 });
  }

  try {
    const res = await fetch(
      `${ENGINE_URL}/api/agents/engine/${expertId}/file/${encodeURIComponent(filename)}`,
    );
    const data = await res.json();
    return NextResponse.json(data);
  } catch (err) {
    console.error('[experts/file-content GET]', err);
    return NextResponse.json({ error: 'Failed to fetch file content' }, { status: 500 });
  }
}

export const dynamic = 'force-dynamic';
