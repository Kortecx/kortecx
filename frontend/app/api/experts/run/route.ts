import { NextRequest, NextResponse } from 'next/server';
import { db, expertRuns, experts } from '@/lib/db';
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

    // Create DB record immediately
    const [run] = await db.insert(expertRuns).values({
      id: runId,
      expertId,
      expertName,
      status: 'running',
      model: model || 'llama3.2:3b',
      engine: engine || 'ollama',
      temperature: String(temperature ?? 0.7),
      maxTokens: maxTokens || 4096,
      systemPrompt: systemPrompt || `You are ${expertName}, a specialized ${role || 'AI'} expert.`,
      userPrompt: userPrompt || `You are running as expert "${expertName}". Provide a demonstration of your capabilities.`,
      startedAt: new Date(),
      metadata: { role, tags },
    }).returning();

    // Fire engine execution in background (don't await)
    fetch(`${ENGINE_URL}/api/experts/engine/${expertId}/execute`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        expertName,
        model: model || 'llama3.2:3b',
        engine: engine || 'ollama',
        temperature: temperature ?? 0.7,
        maxTokens: maxTokens || 4096,
        systemPrompt: systemPrompt || `You are ${expertName}, a specialized ${role || 'AI'} expert.`,
        userPrompt: userPrompt || `You are running as expert "${expertName}". Provide a demonstration of your capabilities.`,
        tags: tags || [role, 'demo', 'auto-run'],
        metadata: { expertId, role, frontendRunId: runId },
        callbackUrl: `${APP_URL}/api/experts/run/complete`,
      }),
    }).catch((err: unknown) => {
      console.error('[experts/run] engine call failed:', err);
    });

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
