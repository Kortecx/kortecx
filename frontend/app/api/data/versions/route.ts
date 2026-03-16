import { NextRequest, NextResponse } from 'next/server';
const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const endpoint = body.restore ? 'file/restore' : 'file/versions';
    const res = await fetch(`${ENGINE_URL}/api/data/${endpoint}`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
    });
    return NextResponse.json(await res.json());
  } catch { return NextResponse.json({ versions: [], error: 'Failed' }); }
}
