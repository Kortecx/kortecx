import { NextRequest, NextResponse } from 'next/server';
import { db, logs } from '@/lib/db';
import { desc, eq, gte } from 'drizzle-orm';

/* GET /api/logs */
export async function GET(req: NextRequest) {

  const { searchParams } = new URL(req.url);
  const level  = searchParams.get('level');
  const limit  = Number(searchParams.get('limit') ?? 100);
  const since  = searchParams.get('since');

  try {
    const rows = await db
      .select()
      .from(logs)
      .where(
        level  ? eq(logs.level, level) :
        since  ? gte(logs.timestamp, new Date(since)) :
        undefined
      )
      .orderBy(desc(logs.timestamp))
      .limit(limit);

    return NextResponse.json({ logs: rows });
  } catch (err) {
    console.error('[logs GET]', err);
    return NextResponse.json({ error: 'Database error' }, { status: 500 });
  }
}

/* POST /api/logs — append a log entry */
export async function POST(req: NextRequest) {

  try {
    const body = await req.json();
    const { level, message, source, metadata, taskId, runId } = body;
    if (!level || !message)
      return NextResponse.json({ error: 'level and message required' }, { status: 400 });

    const [row] = await db.insert(logs).values({
      level, message,
      source:   source   ?? null,
      metadata: metadata ?? null,
      taskId:   taskId   ?? null,
      runId:    runId    ?? null,
    }).returning();

    return NextResponse.json({ log: row }, { status: 201 });
  } catch (err) {
    console.error('[logs POST]', err);
    return NextResponse.json({ error: 'Database error' }, { status: 500 });
  }
}
