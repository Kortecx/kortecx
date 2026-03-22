import { NextRequest, NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { runId, stepId, engine, model, temperature, maxTokens } = body;

    if (!runId || !stepId) {
      return NextResponse.json({ error: 'runId and stepId are required' }, { status: 400 });
    }

    const resp = await fetch(`${ENGINE_URL}/api/metrics/rerun`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        run_id: runId,
        step_id: stepId,
        engine: engine || 'ollama',
        model: model || 'llama3.2:3b',
        temperature: temperature ?? 0.7,
        max_tokens: maxTokens ?? 4096,
      }),
    });

    const data = await resp.json();

    if (!resp.ok) {
      return NextResponse.json(
        { error: data?.error ?? 'Engine rerun failed' },
        { status: resp.status },
      );
    }

    return NextResponse.json(data);
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : 'Rerun failed' },
      { status: 500 },
    );
  }
}
