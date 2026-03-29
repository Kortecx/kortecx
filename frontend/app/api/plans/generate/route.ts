import { NextRequest, NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* POST /api/plans/generate — proxy plan generation to the engine */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { workflowId, workflowSlug, prompt, useGraph, model, engine } = body;

    const engineRes = await fetch(`${ENGINE_URL}/api/plans/generate`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        workflowId: workflowId ?? null,
        workflowSlug: workflowSlug ?? null,
        prompt: prompt ?? null,
        useGraph: useGraph ?? true,
        model: model ?? 'llama3.1:8b',
        engine: engine ?? 'ollama',
      }),
    });

    const data = await engineRes.json();
    return NextResponse.json(data);
  } catch (err) {
    console.error('[plans/generate POST]', err);
    return NextResponse.json({ error: 'Plan generation failed' }, { status: 500 });
  }
}
