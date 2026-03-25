import { NextRequest, NextResponse } from 'next/server';
import { db, logs } from '@/lib/db';
import { desc, eq, gte, and } from 'drizzle-orm';

/* GET /api/logs */
export async function GET(req: NextRequest) {

  const { searchParams } = new URL(req.url);
  const level  = searchParams.get('level');
  const limit  = Number(searchParams.get('limit') ?? 500);
  const since  = searchParams.get('since');
  const source = searchParams.get('source');
  const runId  = searchParams.get('runId');

  try {
    // Build conditions — support combining level + since + source + runId
    const conditions = [];
    if (level)  conditions.push(eq(logs.level, level));
    if (since)  conditions.push(gte(logs.timestamp, new Date(since)));
    if (source) conditions.push(eq(logs.source, source));
    if (runId)  conditions.push(eq(logs.runId, runId));

    const rows = await db
      .select()
      .from(logs)
      .where(conditions.length > 0 ? and(...conditions) : undefined)
      .orderBy(desc(logs.timestamp))
      .limit(limit);

    return NextResponse.json({ logs: rows, total: rows.length });
  } catch (err) {
    console.error('[logs GET]', err);
    return NextResponse.json({ error: 'Database error', logs: [], total: 0 }, { status: 500 });
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
