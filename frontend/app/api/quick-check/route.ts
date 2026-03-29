import { NextRequest, NextResponse } from 'next/server';
import { db, quickChecks } from '@/lib/db';
import { desc, eq } from 'drizzle-orm';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* ── GET — list quick checks ── */
export async function GET(req: NextRequest) {
  try {
    const limit = Number(req.nextUrl.searchParams.get('limit') ?? '50');
    const rows = await db
      .select()
      .from(quickChecks)
      .orderBy(desc(quickChecks.createdAt))
      .limit(limit);

    return NextResponse.json({ checks: rows, total: rows.length });
  } catch (err) {
    console.error('GET /api/quick-check error:', err);
    return NextResponse.json({ error: 'Failed to list quick checks' }, { status: 500 });
  }
}

/* ── POST — create a new quick check record (status: running) ── */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { id, prompt } = body;

    if (!id || !prompt) {
      return NextResponse.json({ error: 'id and prompt are required' }, { status: 400 });
    }

    await db.insert(quickChecks).values({
      id,
      prompt,
      status: 'running',
    }).onConflictDoNothing();

    return NextResponse.json({ id, status: 'running' });
  } catch (err) {
    console.error('POST /api/quick-check error:', err);
    return NextResponse.json({ error: 'Failed to create quick check' }, { status: 500 });
  }
}

/* ── PATCH — update quick check (response, status, etc.) ── */
export async function PATCH(req: NextRequest) {
  try {
    const body = await req.json();
    const { id, ...updates } = body;

    if (!id) {
      return NextResponse.json({ error: 'id is required' }, { status: 400 });
    }

    await db.update(quickChecks).set(updates).where(eq(quickChecks.id, id));
    return NextResponse.json({ updated: id });
  } catch (err) {
    console.error('PATCH /api/quick-check error:', err);
    return NextResponse.json({ error: 'Failed to update quick check' }, { status: 500 });
  }
}

/* ── DELETE — delete a quick check ── */
export async function DELETE(req: NextRequest) {
  try {
    const id = req.nextUrl.searchParams.get('id');
    if (!id) {
      return NextResponse.json({ error: 'id is required' }, { status: 400 });
    }

    await db.delete(quickChecks).where(eq(quickChecks.id, id));

    // Also delete from engine
    try {
      await fetch(`${ENGINE_URL}/api/quick-check/${id}`, { method: 'DELETE' });
    } catch { /* engine may not have it */ }

    return NextResponse.json({ deleted: id });
  } catch (err) {
    console.error('DELETE /api/quick-check error:', err);
    return NextResponse.json({ error: 'Failed to delete quick check' }, { status: 500 });
  }
}
