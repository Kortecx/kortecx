import { NextRequest, NextResponse } from 'next/server';
import { db, expertRuns, experts, tasks } from '@/lib/db';
import { eq, desc } from 'drizzle-orm';
import { logStatus } from '@/lib/status-log';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';
const APP_URL = process.env.NEXT_PUBLIC_APP_URL || 'http://localhost:3000';

/* POST /api/experts/run — start a server-side expert run */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const {
      expertId, expertName, model, engine, temperature, maxTokens,
      systemPrompt, userPrompt, role, tags,
    } = body;

    if (!expertId || !expertName) {
      return NextResponse.json({ error: 'expertId and expertName required' }, { status: 400 });
    }

    const runId = `er-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    const resolvedModel = model || 'llama3.2:3b';
    const resolvedEngine = engine || 'ollama';
    const resolvedSystemPrompt = systemPrompt || `You are ${expertName}, a specialized ${role || 'AI'} expert.`;
    const resolvedUserPrompt = userPrompt || `You are running as expert "${expertName}". Provide a demonstration of your capabilities.`;

    // 1. Create DB record with status 'queued' (not running yet)
    await db.insert(expertRuns).values({
      id: runId,
      expertId,
      expertName,
      status: 'queued',
      model: resolvedModel,
      engine: resolvedEngine,
      temperature: String(temperature ?? 0.7),
      maxTokens: maxTokens || 4096,
      systemPrompt: resolvedSystemPrompt,
      userPrompt: resolvedUserPrompt,
      startedAt: new Date(),
      metadata: { role, tags, frontendRunId: runId },
    }).returning();

    // 2. Await engine execution with timeout (15s)
    let engineAccepted = false;
    try {
      const controller = new AbortController();
      const timeout = setTimeout(() => controller.abort(), 15_000);

      const engineRes = await fetch(`${ENGINE_URL}/api/prism/engine/${expertId}/execute`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        signal: controller.signal,
        body: JSON.stringify({
          expertName,
          model: resolvedModel,
          engine: resolvedEngine,
          temperature: temperature ?? 0.7,
          maxTokens: maxTokens || 4096,
          systemPrompt: resolvedSystemPrompt,
          userPrompt: resolvedUserPrompt,
          tags: tags || [role, 'demo', 'auto-run'],
          metadata: { expertId, role, frontendRunId: runId },
          callbackUrl: `${APP_URL}/api/experts/run/complete`,
        }),
      });

      clearTimeout(timeout);

      if (engineRes.ok) {
        engineAccepted = true;
      } else {
        const errText = await engineRes.text().catch(() => 'Unknown engine error');
        throw new Error(`Engine returned ${engineRes.status}: ${errText}`);
      }
    } catch (engineErr) {
      // Engine unreachable or returned error — mark run as failed
      const errMsg = engineErr instanceof Error ? engineErr.message : 'Engine unreachable';
      await db.update(expertRuns).set({
        status: 'failed',
        errorMessage: errMsg,
        completedAt: new Date(),
      }).where(eq(expertRuns.id, runId));

      // Keep expert as idle
      await db.update(experts).set({ status: 'idle', updatedAt: new Date() })
        .where(eq(experts.id, expertId));

      logStatus('error', `Expert run failed to start: ${expertName} — ${errMsg}`, 'expert', { runId, expertId });
      return NextResponse.json({ runId, status: 'failed', error: errMsg }, { status: 502 });
    }

    // 3. Engine accepted — now set to running
    if (engineAccepted) {
      await db.update(expertRuns).set({ status: 'running' }).where(eq(expertRuns.id, runId));
      await db.update(experts).set({ status: 'running', updatedAt: new Date() })
        .where(eq(experts.id, expertId));
    }

    // 4. Create task queue entry (non-blocking, migration may be pending)
    try {
      const taskId = `task-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
      await db.insert(tasks).values({
        id: taskId,
        name: `Expert Run: ${expertName}`,
        status: 'running',
        priority: 'normal',
        totalSteps: 1,
        currentStep: 0,
        currentExpert: expertName,
        expertId,
        expertRunId: runId,
        progress: 0,
        startedAt: new Date(),
      });
    } catch (taskErr) {
      console.warn('[experts/run] task creation failed (migration may be pending):', taskErr);
    }

    logStatus('info', `Expert run started: ${expertName}`, 'expert', { runId, expertId });
    return NextResponse.json({ runId, status: 'running' }, { status: 202 });
  } catch (err) {
    console.error('[experts/run POST]', err);
    return NextResponse.json({ error: 'Failed to start expert run' }, { status: 500 });
  }
}

/* GET /api/experts/run?id={id}&expertId={id}&status={status} — query expert runs */
export async function GET(req: NextRequest) {
  const id = req.nextUrl.searchParams.get('id');
  const expertId = req.nextUrl.searchParams.get('expertId');
  const status = req.nextUrl.searchParams.get('status');

  try {
    let query = db.select().from(expertRuns).orderBy(desc(expertRuns.createdAt)).$dynamic();

    if (id) {
      query = query.where(eq(expertRuns.id, id));
    }
    if (expertId) {
      query = query.where(eq(expertRuns.expertId, expertId));
    }
    if (status) {
      query = query.where(eq(expertRuns.status, status));
    }

    const rows = await query.limit(50);
    return NextResponse.json({ runs: rows, total: rows.length });
  } catch (err) {
    console.error('[experts/run GET]', err);
    return NextResponse.json({ runs: [], total: 0 });
  }
}

export const dynamic = 'force-dynamic';
