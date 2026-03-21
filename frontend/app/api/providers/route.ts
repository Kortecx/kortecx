import { NextRequest, NextResponse } from 'next/server';
import { db, experts, apiKeys } from '@/lib/db';
import { sql, eq } from 'drizzle-orm';
import { PROVIDERS } from '@/lib/constants';
import { createHash, randomUUID } from 'crypto';
import { encryptToken } from '@/lib/oauth/crypto';
import { logStatus } from '@/lib/status-log';

/* GET /api/providers — returns provider list with live stats and connection status from DB */
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

    // Query api_keys table — find providers with at least one active key
    const activeKeys = await db
      .select({
        providerId: apiKeys.providerId,
        keyPrefix:  apiKeys.keyPrefix,
        keySuffix:  apiKeys.keySuffix,
      })
      .from(apiKeys)
      .where(eq(apiKeys.status, 'active'));

    const connectedSet = new Set<string>();
    const keyDisplayMap = new Map<string, { prefix: string | null; suffix: string | null }>();
    for (const k of activeKeys) {
      connectedSet.add(k.providerId);
      // Keep the most recent key's display info (last wins)
      keyDisplayMap.set(k.providerId, { prefix: k.keyPrefix, suffix: k.keySuffix });
    }

    // Merge live stats + connection status into static provider definitions
    const merged = [];
    for (const p of PROVIDERS) {
      const live = statsMap.get(p.id);
      const hasActiveKey = connectedSet.has(p.id);
      const keyDisplay = keyDisplayMap.get(p.id);
      merged.push({
        id: p.id,
        slug: p.slug,
        name: p.name,
        description: p.description,
        color: p.color,
        connected: hasActiveKey || p.connected,
        apiKeySet: hasActiveKey,
        keyPrefix: keyDisplay?.prefix ?? null,
        keySuffix: keyDisplay?.suffix ?? null,
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
    // Fallback — return static data
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

/* POST /api/providers — connect a provider by storing the API key */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { providerId, apiKey } = body;

    if (!providerId || typeof providerId !== 'string') {
      return NextResponse.json({ error: 'providerId is required' }, { status: 400 });
    }
    if (!apiKey || typeof apiKey !== 'string') {
      return NextResponse.json({ error: 'apiKey is required' }, { status: 400 });
    }

    // SHA-256 hash for key identification / deduplication
    const keyHash = createHash('sha256').update(apiKey).digest('hex');

    // Display fragments
    const keyPrefix = apiKey.slice(0, 8);
    const keySuffix = apiKey.length > 4 ? apiKey.slice(-4) : null;

    // AES-256-GCM encrypt for secure storage
    const encryptedKey = encryptToken(apiKey);

    // Revoke any existing active keys for this provider before inserting the new one
    await db
      .update(apiKeys)
      .set({ status: 'revoked' })
      .where(
        sql`${apiKeys.providerId} = ${providerId} AND ${apiKeys.status} = 'active'`
      );

    // Insert new key
    await db.insert(apiKeys).values({
      id: randomUUID(),
      providerId,
      keyHash,
      keyPrefix,
      keySuffix,
      encryptedKey,
      status: 'active',
    });

    logStatus('info', `API key added for ${providerId}`, 'provider', { providerId });
    return NextResponse.json({
      success: true,
      providerId,
      status: 'connected',
      apiKeySet: true,
    });
  } catch (err) {
    console.error('[POST /api/providers]', err);
    logStatus('error', `API key storage failed: ${err instanceof Error ? err.message : 'Unknown'}`, 'provider', { error: err instanceof Error ? err.message : 'Unknown' });
    return NextResponse.json({ error: 'Failed to store API key' }, { status: 500 });
  }
}

/* DELETE /api/providers?providerId=<id> — soft-delete (revoke) a provider's keys */
export async function DELETE(req: NextRequest) {
  try {
    const providerId = req.nextUrl.searchParams.get('providerId');

    if (!providerId) {
      return NextResponse.json({ error: 'providerId query parameter is required' }, { status: 400 });
    }

    // Soft-delete: set status to 'revoked' for all active keys of this provider
    await db
      .update(apiKeys)
      .set({ status: 'revoked' })
      .where(
        sql`${apiKeys.providerId} = ${providerId} AND ${apiKeys.status} = 'active'`
      );

    logStatus('info', `API key revoked for ${providerId}`, 'provider', { providerId });
    return NextResponse.json({
      success: true,
      providerId,
      status: 'disconnected',
      apiKeySet: false,
    });
  } catch (err) {
    console.error('[DELETE /api/providers]', err);
    logStatus('error', `API key revocation failed: ${err instanceof Error ? err.message : 'Unknown'}`, 'provider', { error: err instanceof Error ? err.message : 'Unknown' });
    return NextResponse.json({ error: 'Failed to revoke API key' }, { status: 500 });
  }
}
