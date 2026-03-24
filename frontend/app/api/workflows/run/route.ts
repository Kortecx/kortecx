import { NextRequest, NextResponse } from 'next/server';
import { db, workflowRuns } from '@/lib/db';
import { logStatus } from '@/lib/status-log';

/* POST /api/workflows/run — Create a run record in NeonDB.
 * Execution is triggered via WebSocket (workflow.execute event).
 * This route only persists the run record for reliable DB tracking. */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { name, goalFileUrl, inputFileUrls, steps, workflowId } = body;

    if (!name || !name.trim()) {
      return NextResponse.json({ error: 'Workflow name is required' }, { status: 400 });
    }
    if (!steps || steps.length === 0) {
      return NextResponse.json({ error: 'At least one step is required' }, { status: 400 });
    }

    const runId = body.runId || `run-${Date.now()}`;
    const expertChain = steps
      .map((s: Record<string, unknown>) => s.expertId || 'unknown')
      .filter(Boolean) as string[];

    // Persist run record to database
    await db.insert(workflowRuns).values({
      id:             runId,
      workflowId:     workflowId || 'adhoc',
      workflowName:   name.trim(),
      status:         'running',
      startedAt:      new Date(),
      input:          goalFileUrl || '',
      expertChain,
      metadata:       {
        inputFileUrls: inputFileUrls || [],
        stepsCount: steps.length,
        parallelSteps: steps.filter((s: Record<string, unknown>) => s.connectionType === 'parallel').length,
      },
    });

    logStatus('info', `Workflow run created: ${name}`, 'workflow', { runId, workflowId: workflowId || 'adhoc', stepsCount: steps.length });
    return NextResponse.json({ runId, status: 'created' }, { status: 202 });
  } catch (err) {
    console.error('[workflows/run POST]', err);
    logStatus('error', `Workflow run failed: ${err instanceof Error ? err.message : 'Unknown'}`, 'workflow', { error: err instanceof Error ? err.message : 'Unknown' });
    return NextResponse.json({ error: 'Failed to start workflow' }, { status: 500 });
  }
}
