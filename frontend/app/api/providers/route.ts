import { NextRequest, NextResponse } from 'next/server';
import { db, experts } from '@/lib/db';
import { sql } from 'drizzle-orm';
import { PROVIDERS } from '@/lib/constants';

/* GET /api/providers — returns provider list with live stats from DB when available */
export async function GET() {
  try {
    // Get per-provider stats from experts table
    const providerStats = await db
      .select({
        providerId:   experts.providerId,
        providerName: experts.providerName,
        expertCount:  sql<number>`COUNT(*)`,
        totalRuns:    sql<number>`COALESCE(SUM(${experts.totalRuns}), 0)`,
        avgLatency:   sql<number>`COALESCE(AVG(${experts.avgLatencyMs}), 0)`,
      })
      .from(experts)
      .groupBy(experts.providerId, experts.providerName);

    const statsMap = new Map<string, { expertCount: number; totalRuns: number; avgLatency: number }>();
    for (const s of providerStats) {
      statsMap.set(s.providerId, {
        expertCount: Number(s.expertCount),
        totalRuns: Number(s.totalRuns),
        avgLatency: Number(s.avgLatency),
      });
    }

    // Merge live stats into static provider definitions
    const merged = [];
    for (const p of PROVIDERS) {
      const live = statsMap.get(p.id);
      merged.push({
        id: p.id,
        slug: p.slug,
        name: p.name,
        description: p.description,
        color: p.color,
        connected: p.connected,
        apiKeySet: p.apiKeySet,
        status: p.status,
        latencyMs: live ? Math.round(live.avgLatency) : p.latencyMs,
        expertCount: live ? live.expertCount : 0,
        totalRuns: live ? live.totalRuns : 0,
        models: p.models,
        monthlyTokensUsed: p.monthlyTokensUsed,
        monthlyTokenLimit: p.monthlyTokenLimit,
      });
    }

    return NextResponse.json({ providers: merged });
  } catch {
    // Fallback
    return NextResponse.json({
      providers: PROVIDERS.map(p => ({
        id: p.id,
        slug: p.slug,
        name: p.name,
        description: p.description,
        color: p.color,
        connected: p.connected,
        apiKeySet: p.apiKeySet,
        status: p.status,
        latencyMs: p.latencyMs,
        models: p.models,
        monthlyTokensUsed: p.monthlyTokensUsed,
        monthlyTokenLimit: p.monthlyTokenLimit,
      })),
    });
  }
}

/* POST /api/providers — connect a provider (store API key) */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { providerId, apiKey } = body;

    if (!providerId) {
      return NextResponse.json({ error: 'providerId is required' }, { status: 400 });
    }

    // TODO: In production, encrypt and store the API key
    // For now, just acknowledge the connection
    return NextResponse.json({
      success: true,
      providerId,
      status: 'connected',
      apiKeySet: !!apiKey,
    });
  } catch {
    return NextResponse.json({ error: 'Invalid request body' }, { status: 400 });
  }
}
