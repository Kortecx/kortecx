import { NextRequest, NextResponse } from 'next/server';
import { desc } from 'drizzle-orm';
import { db } from '@/lib/db';
import { modelComparisons } from '@/lib/db/schema';

export async function GET(req: NextRequest) {
  try {
    const url = new URL(req.url);
    const limit = Math.min(parseInt(url.searchParams.get('limit') || '50', 10), 200);
    const offset = parseInt(url.searchParams.get('offset') || '0', 10);

    const rows = await db
      .select()
      .from(modelComparisons)
      .orderBy(desc(modelComparisons.createdAt))
      .limit(limit)
      .offset(offset);

    return NextResponse.json({ comparisons: rows, count: rows.length });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : 'Unknown error';
    return NextResponse.json({ error: msg }, { status: 500 });
  }
}

export async function POST(req: NextRequest) {
  try {
    const body = await req.json();

    const row = {
      id: crypto.randomUUID(),
      runId: body.runId || 'compare-standalone',
      stepId: body.stepId || 'model-compare',
      originalModel: body.originalModel || body.model_a?.model || '',
      originalEngine: body.originalEngine || body.model_a?.engine || null,
      originalTokens: body.originalTokens ?? body.model_a?.tokens ?? 0,
      originalDurationMs: body.originalDurationMs ?? body.model_a?.duration_ms ?? 0,
      originalResponse: body.originalResponse || body.model_a?.response || null,
      comparisonModel: body.comparisonModel || body.model_b?.model || '',
      comparisonEngine: body.comparisonEngine || body.model_b?.engine || null,
      comparisonTokens: body.comparisonTokens ?? body.model_b?.tokens ?? 0,
      comparisonDurationMs: body.comparisonDurationMs ?? body.model_b?.duration_ms ?? 0,
      comparisonResponse: body.comparisonResponse || body.model_b?.response || null,
      temperature: body.temperature?.toString() ?? '0.7',
      prompt: body.prompt || null,
      systemPrompt: body.systemPrompt || null,
      documentNames: body.documentNames || null,
      documentContent: body.documentContent || null,
      originalTokensPerSec: body.model_a?.tokens_per_sec?.toString() ?? null,
      comparisonTokensPerSec: body.model_b?.tokens_per_sec?.toString() ?? null,
    };

    const [inserted] = await db.insert(modelComparisons).values(row).returning();
    return NextResponse.json(inserted, { status: 201 });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : 'Unknown error';
    return NextResponse.json({ error: msg }, { status: 500 });
  }
}
