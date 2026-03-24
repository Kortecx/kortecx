import { NextRequest, NextResponse } from 'next/server';
import { db, workflowRuns, stepExecutions, workflows } from '@/lib/db';
import { desc, eq } from 'drizzle-orm';

/* ── POST — upsert run (called by engine _sync_to_frontend) ── */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const {
      id, workflowId, workflowName, status,
      startedAt, completedAt, totalTokensUsed,
      durationSec, expertChain, errorMessage,
    } = body;

    if (!id || !workflowId) {
      return NextResponse.json({ error: 'id and workflowId are required' }, { status: 400 });
    }

    // Upsert: insert or update on conflict
    const values = {
      id,
      workflowId,
      workflowName: workflowName ?? 'Unnamed',
      status: status ?? 'running',
      startedAt: startedAt ? new Date(startedAt) : new Date(),
      completedAt: completedAt ? new Date(completedAt) : null,
      totalTokensUsed: totalTokensUsed ?? 0,
      durationSec: durationSec != null ? Math.round(durationSec) : null,
      expertChain: expertChain ?? [],
      errorMessage: errorMessage ?? null,
    };

    await db.insert(workflowRuns).values(values).onConflictDoUpdate({
      target: workflowRuns.id,
      set: {
        status: values.status,
        completedAt: values.completedAt,
        totalTokensUsed: values.totalTokensUsed,
        durationSec: values.durationSec,
        expertChain: values.expertChain,
        errorMessage: values.errorMessage,
      },
    });

    // Propagate terminal status to the parent workflow row
    if (['completed', 'failed', 'cancelled'].includes(values.status)) {
      await db.update(workflows)
        .set({ status: values.status, updatedAt: new Date() })
        .where(eq(workflows.id, workflowId))
        .catch((e: unknown) => console.warn('[workflow runs POST] failed to update workflow status', e));
    }

    return NextResponse.json({ ok: true, id });
  } catch (err) {
    console.error('[workflow runs POST]', err);
    return NextResponse.json({ error: 'Failed to upsert run' }, { status: 500 });
  }
}

export async function GET(req: NextRequest) {

  const { searchParams } = new URL(req.url);
  const workflowId = searchParams.get('workflowId');
  const limit = Number(searchParams.get('limit') ?? 50);

  try {
    const query = db.select().from(workflowRuns);
    if (workflowId) query.where(eq(workflowRuns.workflowId, workflowId));
    const rows = await query.orderBy(desc(workflowRuns.createdAt)).limit(limit);

    return NextResponse.json({ runs: rows, total: rows.length });
  } catch (err) {
    console.error('[workflow runs GET]', err);
    return NextResponse.json({ runs: [], total: 0 });
  }
}

export async function DELETE(req: NextRequest) {
  const { searchParams } = new URL(req.url);
  const id = searchParams.get('id');

  if (!id) {
    return NextResponse.json({ error: 'Run id is required' }, { status: 400 });
  }

  try {
    // Delete related step executions first
    await db.delete(stepExecutions).where(eq(stepExecutions.runId, id));
    const [deleted] = await db.delete(workflowRuns).where(eq(workflowRuns.id, id)).returning();

    if (!deleted) {
      return NextResponse.json({ error: 'Run not found' }, { status: 404 });
    }

    return NextResponse.json({ deleted: true, id });
  } catch (err) {
    console.error('[workflow runs DELETE]', err);
    return NextResponse.json({ error: 'Delete failed' }, { status: 500 });
  }
}
