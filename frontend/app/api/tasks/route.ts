import { NextRequest, NextResponse } from 'next/server';
import { db, tasks } from '@/lib/db';
import { desc, eq, sql } from 'drizzle-orm';
import { nanoid } from './nanoid';
import { logStatus } from '@/lib/status-log';

/* GET /api/tasks — list tasks with optional status filter */
export async function GET(req: NextRequest) {

  const { searchParams } = new URL(req.url);
  const status = searchParams.get('status');
  const limit  = Number(searchParams.get('limit') ?? 50);

  try {
    const rows = await db
      .select()
      .from(tasks)
      .where(status ? eq(tasks.status, status) : undefined)
      .orderBy(desc(tasks.createdAt))
      .limit(limit);

    return NextResponse.json({ tasks: rows });
  } catch (err) {
    console.error('[tasks GET]', err);
    return NextResponse.json({ error: 'Database error' }, { status: 500 });
  }
}

/* POST /api/tasks — create / queue a new task */
export async function POST(req: NextRequest) {

  try {
    const body = await req.json();
    const { name, workflowId, workflowName, priority, totalSteps, estimatedTokens, input } = body;

    if (!name) return NextResponse.json({ error: 'name is required' }, { status: 400 });

    const [row] = await db.insert(tasks).values({
      id: `task-${nanoid()}`,
      name,
      workflowId,
      workflowName,
      status:          'queued',
      priority:        priority ?? 'normal',
      totalSteps:      totalSteps ?? 1,
      estimatedTokens: estimatedTokens ?? null,
      input:           input ?? null,
    }).returning();

    logStatus('info', `Task created: ${name}`, 'task', { id: row.id, workflowId });
    return NextResponse.json({ task: row }, { status: 201 });
  } catch (err) {
    console.error('[tasks POST]', err);
    logStatus('error', `Task creation failed: ${err instanceof Error ? err.message : 'Unknown'}`, 'task', { error: err instanceof Error ? err.message : 'Unknown' });
    return NextResponse.json({ error: 'Database error' }, { status: 500 });
  }
}

/* PATCH /api/tasks — update task status / progress */
export async function PATCH(req: NextRequest) {

  try {
    const body = await req.json();
    const { id, ...updates } = body;
    if (!id) return NextResponse.json({ error: 'id is required' }, { status: 400 });

    // Auto-set timestamps
    if (updates.status === 'running' && !updates.startedAt) {
      updates.startedAt = new Date();
    }
    if (['completed','failed','cancelled'].includes(updates.status) && !updates.completedAt) {
      updates.completedAt = new Date();
    }
    updates.updatedAt = new Date();

    const [row] = await db.update(tasks)
      .set(updates)
      .where(eq(tasks.id, id))
      .returning();

    if (!row) return NextResponse.json({ error: 'Task not found' }, { status: 404 });
    logStatus('info', `Task updated: ${id}`, 'task', { id, status: updates.status });
    return NextResponse.json({ task: row });
  } catch (err) {
    console.error('[tasks PATCH]', err);
    logStatus('error', `Task update failed: ${err instanceof Error ? err.message : 'Unknown'}`, 'task', { error: err instanceof Error ? err.message : 'Unknown' });
    return NextResponse.json({ error: 'Database error' }, { status: 500 });
  }
}
