import { NextRequest, NextResponse } from 'next/server';
import { db, metrics, tasks, workflowRuns, alerts } from '@/lib/db';
import { desc, eq, gte, count, sql } from 'drizzle-orm';

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
