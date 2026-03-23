import { NextRequest, NextResponse } from 'next/server';
import { db, workflowRuns, workflows } from '@/lib/db';
import { eq } from 'drizzle-orm';

const ENGINE_URL = process.env.ENGINE_URL || 'http://localhost:8000';

/* POST /api/workflows/restart — Restart a completed/failed/cancelled workflow */
export async function POST(req: NextRequest) {
  try {
    const { runId, workflowId } = await req.json();

    if (!runId) {
      return NextResponse.json({ error: 'runId is required' }, { status: 400 });
    }

    // Signal restart to the engine orchestrator
    const engineResp = await fetch(`${ENGINE_URL}/api/orchestrator/runs/${runId}/restart`, {
      method: 'POST',
    });
    const data = await engineResp.json();

    if (data.error) {
      return NextResponse.json({ error: data.error }, { status: 400 });
    }

    // Create a new run record for the restart
    const newRunId = `run-${Date.now()}`;
    const oldRun = await db.select().from(workflowRuns).where(eq(workflowRuns.id, runId)).limit(1);
    if (oldRun.length > 0) {
      await db.insert(workflowRuns).values({
        id:           newRunId,
        workflowId:   oldRun[0].workflowId,
        workflowName: oldRun[0].workflowName,
        status:       'running',
        startedAt:    new Date(),
        input:        oldRun[0].input,
        expertChain:  oldRun[0].expertChain,
        metadata:     oldRun[0].metadata,
      });
    }

    // Update workflow status to running
    if (workflowId) {
      await db.update(workflows)
        .set({ status: 'running', lastRunAt: new Date() })
        .where(eq(workflows.id, workflowId));
    }

    return NextResponse.json({ runId: newRunId, status: 'restarting', message: 'Workflow restart initiated' });
  } catch (err) {
    console.error('[workflows/restart POST]', err);
    return NextResponse.json({ error: 'Failed to restart workflow' }, { status: 500 });
  }
}
