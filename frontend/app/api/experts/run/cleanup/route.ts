import { NextResponse } from 'next/server';
import { db, expertRuns, experts } from '@/lib/db';
import { and, eq, lt } from 'drizzle-orm';

const STALE_RUNNING_MS = 5 * 60 * 1000;  // 5 minutes
const STALE_QUEUED_MS  = 60 * 1000;       // 1 minute

/* POST /api/experts/run/cleanup — mark stale running/queued runs as failed */
export async function POST() {
  try {
    let totalCleaned = 0;

    // 1. Clean up runs stuck in 'running' for over 5 minutes
    const runningCutoff = new Date(Date.now() - STALE_RUNNING_MS);
    const staleRunning = await db.update(expertRuns).set({
      status: 'failed',
      errorMessage: 'Run timed out — engine did not respond within 5 minutes',
      completedAt: new Date(),
    }).where(
      and(
        eq(expertRuns.status, 'running'),
        lt(expertRuns.startedAt, runningCutoff),
      ),
    ).returning();
    totalCleaned += staleRunning.length;

    // 2. Clean up runs stuck in 'queued' for over 1 minute
    const queuedCutoff = new Date(Date.now() - STALE_QUEUED_MS);
    const staleQueued = await db.update(expertRuns).set({
      status: 'failed',
      errorMessage: 'Run never started — engine may be offline',
      completedAt: new Date(),
    }).where(
      and(
        eq(expertRuns.status, 'queued'),
        lt(expertRuns.createdAt, queuedCutoff),
      ),
    ).returning();
    totalCleaned += staleQueued.length;

    // 3. Reset expert status for all affected experts back to 'idle'
    const affectedExperts = new Set<string>();
    for (const run of [...staleRunning, ...staleQueued]) {
      if (run.expertId) affectedExperts.add(run.expertId);
    }
    for (const eid of affectedExperts) {
      await db.update(experts).set({
        status: 'idle',
        updatedAt: new Date(),
      }).where(eq(experts.id, eid));
    }

    return NextResponse.json({
      cleaned: totalCleaned,
      staleRunning: staleRunning.length,
      staleQueued: staleQueued.length,
      expertsReset: affectedExperts.size,
    });
  } catch (err) {
    console.error('[experts/run/cleanup]', err);
    return NextResponse.json({ error: 'Cleanup failed' }, { status: 500 });
  }
}

export const dynamic = 'force-dynamic';
