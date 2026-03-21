import { NextRequest, NextResponse } from 'next/server';
import { db } from '@/lib/db';
import { stepExecutions } from '@/lib/db/schema';
import { desc, eq, and } from 'drizzle-orm';

export async function GET(req: NextRequest) {
  try {
    const { searchParams } = new URL(req.url);
    const runId = searchParams.get('runId');
    const stepId = searchParams.get('stepId');
    const limit = parseInt(searchParams.get('limit') || '100');

    const conditions = [];
    if (runId) conditions.push(eq(stepExecutions.runId, runId));
    if (stepId) conditions.push(eq(stepExecutions.stepId, stepId));

    const rows = await db.select().from(stepExecutions)
      .where(conditions.length > 0 ? and(...conditions) : undefined)
      .orderBy(desc(stepExecutions.createdAt))
      .limit(limit);

    return NextResponse.json({ executions: rows, total: rows.length });
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : 'Failed to fetch executions' },
      { status: 500 },
    );
  }
}

export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const {
      runId, workflowId, stepId, agentId, expertId, stepName,
      status, model, engine, tokensUsed, promptTokens, completionTokens,
      durationMs, queueWaitMs, inferenceMs,
      cpuPercent, gpuPercent, memoryMb,
      promptPreview, responsePreview, errorMessage, metadata,
    } = body;

    if (!runId || !stepId) {
      return NextResponse.json({ error: 'runId and stepId are required' }, { status: 400 });
    }

    const id = `se-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

    const [row] = await db.insert(stepExecutions).values({
      id,
      runId,
      workflowId: workflowId || null,
      stepId,
      agentId: agentId || null,
      expertId: expertId || null,
      stepName: stepName || null,
      status: status || 'pending',
      model: model || null,
      engine: engine || null,
      tokensUsed: tokensUsed || 0,
      promptTokens: promptTokens || 0,
      completionTokens: completionTokens || 0,
      durationMs: durationMs || 0,
      queueWaitMs: queueWaitMs || 0,
      inferenceMs: inferenceMs || 0,
      cpuPercent: String(cpuPercent || 0),
      gpuPercent: String(gpuPercent || 0),
      memoryMb: String(memoryMb || 0),
      promptPreview: promptPreview ? String(promptPreview).slice(0, 500) : null,
      responsePreview: responsePreview ? String(responsePreview).slice(0, 500) : null,
      errorMessage: errorMessage || null,
      metadata: metadata || null,
      startedAt: body.startedAt ? new Date(body.startedAt) : null,
      completedAt: body.completedAt ? new Date(body.completedAt) : null,
    }).returning();

    return NextResponse.json({ execution: row });
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : 'Failed to persist execution' },
      { status: 500 },
    );
  }
}

export async function PATCH(req: NextRequest) {
  try {
    const body = await req.json();
    const { id, ...updates } = body;

    if (!id) {
      return NextResponse.json({ error: 'id is required' }, { status: 400 });
    }

    // Build update object, only include defined fields
    const updateFields: Record<string, unknown> = {};
    if (updates.status !== undefined) updateFields.status = updates.status;
    if (updates.tokensUsed !== undefined) updateFields.tokensUsed = updates.tokensUsed;
    if (updates.promptTokens !== undefined) updateFields.promptTokens = updates.promptTokens;
    if (updates.completionTokens !== undefined) updateFields.completionTokens = updates.completionTokens;
    if (updates.durationMs !== undefined) updateFields.durationMs = updates.durationMs;
    if (updates.queueWaitMs !== undefined) updateFields.queueWaitMs = updates.queueWaitMs;
    if (updates.inferenceMs !== undefined) updateFields.inferenceMs = updates.inferenceMs;
    if (updates.cpuPercent !== undefined) updateFields.cpuPercent = String(updates.cpuPercent);
    if (updates.gpuPercent !== undefined) updateFields.gpuPercent = String(updates.gpuPercent);
    if (updates.memoryMb !== undefined) updateFields.memoryMb = String(updates.memoryMb);
    if (updates.responsePreview !== undefined) updateFields.responsePreview = String(updates.responsePreview).slice(0, 500);
    if (updates.errorMessage !== undefined) updateFields.errorMessage = updates.errorMessage;
    if (updates.completedAt !== undefined) updateFields.completedAt = new Date(updates.completedAt);

    const [row] = await db.update(stepExecutions)
      .set(updateFields)
      .where(eq(stepExecutions.id, id))
      .returning();

    return NextResponse.json({ execution: row });
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : 'Failed to update execution' },
      { status: 500 },
    );
  }
}
