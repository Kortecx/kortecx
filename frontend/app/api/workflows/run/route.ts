import { NextRequest, NextResponse } from 'next/server';
import { db, workflowRuns } from '@/lib/db';
import { eq } from 'drizzle-orm';
import { logStatus } from '@/lib/status-log';

const ENGINE_URL = process.env.ENGINE_URL || 'http://localhost:8000';

/* POST /api/workflows/run — Execute a workflow via the engine orchestrator */
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

    const runId = `run-${Date.now()}`;
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

    // Forward to engine orchestrator with all step fields
    const engineResp = await fetch(`${ENGINE_URL}/api/orchestrator/execute`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        workflowId: workflowId || runId,
        name: name.trim(),
        goalFileUrl: goalFileUrl || '',
        inputFileUrls: inputFileUrls || [],
        steps: steps.map((s: Record<string, unknown>, i: number) => ({
          stepId: s.stepId || `step-${i + 1}`,
          expertId: s.expertId || null,
          taskDescription: s.taskDescription || '',
          systemInstructions: s.systemInstructions || '',
          voiceCommand: s.voiceCommand || '',
          fileLocations: s.fileLocations || [],
          stepFileNames: s.stepFileNames || [],
          stepImageNames: s.stepImageNames || [],
          modelSource: s.modelSource || 'provider',
          localModel: s.localModel || null,
          temperature: s.temperature ?? 0.7,
          maxTokens: s.maxTokens ?? 4096,
          connectionType: s.connectionType || 'sequential',
        })),
      }),
    });

    const data = await engineResp.json();

    if (!engineResp.ok) {
      // Update run status to failed
      await db.update(workflowRuns)
        .set({ status: 'failed', errorMessage: data.detail || data.error || 'Engine error' })
        .where(eq(workflowRuns.id, runId));

      return NextResponse.json(
        { error: data.detail || data.error || 'Engine error' },
        { status: engineResp.status },
      );
    }

    logStatus('info', `Workflow run started: ${name}`, 'workflow', { runId, workflowId: workflowId || 'adhoc', stepsCount: steps.length });
    return NextResponse.json({ ...data, runId }, { status: 202 });
  } catch (err) {
    console.error('[workflows/run POST]', err);
    logStatus('error', `Workflow run failed: ${err instanceof Error ? err.message : 'Unknown'}`, 'workflow', { error: err instanceof Error ? err.message : 'Unknown' });
    return NextResponse.json({ error: 'Failed to start workflow' }, { status: 500 });
  }
}
