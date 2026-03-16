import { NextRequest, NextResponse } from 'next/server';
const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const res = await fetch(`${ENGINE_URL}/api/data/file/schema`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
    });
    return NextResponse.json(await res.json());
  } catch { return NextResponse.json({ columns: [], error: 'Failed' }); }
}
