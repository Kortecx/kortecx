import { NextRequest, NextResponse } from 'next/server';
import { db, metrics, alerts, logs } from '@/lib/db';
import { desc, eq, count } from 'drizzle-orm';

export async function GET(req: NextRequest) {

  try {
    const [snap, recentAlerts, recentLogs, unack] = await Promise.all([
      db.select().from(metrics).orderBy(desc(metrics.capturedAt)).limit(1),
      db.select().from(alerts).orderBy(desc(alerts.createdAt)).limit(20),
      db.select().from(logs).orderBy(desc(logs.timestamp)).limit(50),
      db.select({ count: count() }).from(alerts).where(eq(alerts.acknowledged, false)),
    ]);

    const m = snap[0];
    return NextResponse.json({
      system: {
        activeAgents: m?.activeAgents  ?? 0,
        avgLatencyMs: m?.avgLatencyMs  ?? 0,
        successRate:  Number(m?.successRate ?? 0),
        errorCount:   m?.errorCount    ?? 0,
        costUsd:      Number(m?.costUsd ?? 0),
        tokensUsed:   m?.tokensUsed    ?? 0,
        timestamp:    m?.capturedAt ?? new Date().toISOString(),
      },
      alerts:           recentAlerts,
      logs:             recentLogs,
      unackedAlertCount: unack[0]?.count ?? 0,
    });
  } catch (err) {
    console.error('[monitoring GET]', err);
    return NextResponse.json({ error: 'Database error' }, { status: 500 });
  }
}
