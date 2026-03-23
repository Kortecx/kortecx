import { NextRequest, NextResponse } from 'next/server';
import { db, workflowRuns, workflows } from '@/lib/db';
import { eq } from 'drizzle-orm';

const ENGINE_URL = process.env.ENGINE_URL || 'http://localhost:8000';

/* POST /api/workflows/stop — Cancel a running workflow */
export async function POST(req: NextRequest) {
  try {
    const { runId, workflowId } = await req.json();

    if (!runId) {
      return NextResponse.json({ error: 'runId is required' }, { status: 400 });
    }

    // Signal cancellation to the engine orchestrator
    const engineResp = await fetch(`${ENGINE_URL}/api/orchestrator/runs/${runId}/cancel`, {
      method: 'POST',
    });
    const data = await engineResp.json();

    if (data.error) {
      return NextResponse.json({ error: data.error }, { status: 400 });
    }

    // Update run status in DB
    await db.update(workflowRuns)
      .set({ status: 'cancelled', completedAt: new Date() })
      .where(eq(workflowRuns.id, runId));

    // Reset workflow status to idle
    if (workflowId) {
      await db.update(workflows)
        .set({ status: 'idle' })
        .where(eq(workflows.id, workflowId));
    }

    return NextResponse.json({ runId, status: 'cancelled', message: 'Workflow cancelled' });
  } catch (err) {
    console.error('[workflows/stop POST]', err);
    return NextResponse.json({ error: 'Failed to stop workflow' }, { status: 500 });
  }
}
