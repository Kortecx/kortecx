import { NextRequest, NextResponse } from 'next/server';
import { db, metrics, tasks, workflowRuns, alerts } from '@/lib/db';
import { desc, eq, gte, count } from 'drizzle-orm';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* GET /api/metrics — live dashboard metrics */
export async function GET(req: NextRequest) {

  try {
    const since24h = new Date(Date.now() - 24 * 60 * 60 * 1000);

    // Run queries in parallel
    const [
      latestMetrics,
      runningTasks,
      queuedTasks,
      todayRuns,
      unackAlerts,
      recentMetrics,
    ] = await Promise.all([
      /* Latest snapshot */
      db.select().from(metrics).orderBy(desc(metrics.capturedAt)).limit(1),

      /* Currently running tasks */
      db.select({ count: count() }).from(tasks).where(eq(tasks.status, 'running')),

      /* Queued tasks */
      db.select({ count: count() }).from(tasks).where(eq(tasks.status, 'queued')),

      /* Workflow runs today */
      db.select({ count: count() }).from(workflowRuns)
        .where(gte(workflowRuns.createdAt, since24h)),

      /* Unacknowledged alerts */
      db.select({ count: count() }).from(alerts)
        .where(eq(alerts.acknowledged, false)),

      /* Last 24 metric snapshots for sparklines */
      db.select().from(metrics)
        .orderBy(desc(metrics.capturedAt))
        .limit(24),
    ]);

    const snap = latestMetrics[0];

    // Try to get live engine metrics
    let live = null;
    try {
      const resp = await fetch(`${ENGINE_URL}/api/metrics/live`, {
        signal: AbortSignal.timeout(3_000),
      });
      if (resp.ok) live = await resp.json();
    } catch { /* engine offline — continue with DB data only */ }

    // Auto-capture metric snapshot if stale (> 1 minute)
    const isStale = !snap || (Date.now() - new Date(snap.capturedAt).getTime() > 60_000);
    if (isStale && live) {
      try {
        await db.insert(metrics).values({
          activeAgents: live.activeAgents ?? 0,
          tasksCompleted: live.tasksCompleted ?? 0,
          tokensUsed: live.tokensUsed ?? 0,
          avgLatencyMs: live.avgLatencyMs ?? 0,
          successRate: String(live.successRate ?? 0),
          costUsd: '0',
          errorCount: live.tasksFailed ?? 0,
        });
      } catch { /* ignore auto-capture failures */ }
    }

    return NextResponse.json({
      activeAgents:   snap?.activeAgents ?? 0,
      tasksToday:     todayRuns[0]?.count ?? 0,
      tokensUsedToday:snap?.tokensUsed ?? 0,
      tokenBudgetDaily: 5_000_000,
      successRate:    Number(snap?.successRate ?? 0),
      avgLatencyMs:   snap?.avgLatencyMs ?? 0,
      costToday:      Number(snap?.costUsd ?? 0),
      errorCount:     snap?.errorCount ?? 0,
      runningTasks:   runningTasks[0]?.count ?? 0,
      queuedTasks:    queuedTasks[0]?.count ?? 0,
      unackAlerts:    unackAlerts[0]?.count ?? 0,
      sparkline:      recentMetrics.reverse(),
      live,
    });
  } catch (err) {
    console.error('[metrics GET]', err);
    return NextResponse.json({ error: 'Database error' }, { status: 500 });
  }
}

/* POST /api/metrics — record a new snapshot (called by cron / system) */
export async function POST(req: NextRequest) {

  try {
    const body = await req.json();
    const [row] = await db.insert(metrics).values(body).returning();
    return NextResponse.json({ metric: row }, { status: 201 });
  } catch (err) {
    console.error('[metrics POST]', err);
    return NextResponse.json({ error: 'Database error' }, { status: 500 });
  }
}
