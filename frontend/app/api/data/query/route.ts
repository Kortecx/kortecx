import { NextRequest, NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* POST /api/data/query — query a data file via DuckDB */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const res = await fetch(`${ENGINE_URL}/api/data/file/query`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!res.ok) {
      return NextResponse.json({ error: 'Query failed', rows: [], columns: [], totalRows: 0 });
    }
    return NextResponse.json(await res.json());
  } catch (err) {
    console.error('[data/query POST]', err);
    return NextResponse.json({ error: 'Query failed', rows: [], columns: [], totalRows: 0 });
  }
}
