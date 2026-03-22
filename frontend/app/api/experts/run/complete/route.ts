import { NextRequest, NextResponse } from 'next/server';
import { db, expertRuns, experts, assets, lineage, tasks } from '@/lib/db';
import { eq } from 'drizzle-orm';
import { logStatus } from '@/lib/status-log';

/* POST /api/experts/run/complete — callback from engine when expert run finishes */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const {
      runId, expertId, expertName, status, responseText,
      tokensUsed, durationMs, model, engine, errorMessage, artifacts,
      metadata: callbackMeta,
    } = body;

    if (!runId) {
      return NextResponse.json({ error: 'runId required' }, { status: 400 });
    }

    // Match by frontendRunId from metadata (precise), fallback to expertId search
    const frontendRunId = callbackMeta?.frontendRunId || body.metadata?.frontendRunId;
    let dbRunId: string | undefined;

    if (frontendRunId) {
      // Precise match by the ID we assigned
      const [directMatch] = await db.select().from(expertRuns)
        .where(eq(expertRuns.id, frontendRunId))
        .limit(1);
      dbRunId = directMatch?.id;
    }

    if (!dbRunId) {
      // Fallback: find most recent running/queued run for this expert
      const candidates = await db.select().from(expertRuns)
        .where(eq(expertRuns.expertId, expertId))
        .orderBy(expertRuns.createdAt)
        .limit(10);
      const match = candidates.find(
        (r: { status: string }) => r.status === 'running' || r.status === 'queued',
      );
      dbRunId = match?.id;
    }

    if (!dbRunId) {
      console.warn('[experts/run/complete] No matching run found for expert:', expertId, 'engineRunId:', runId);
      return NextResponse.json({ warning: 'No matching run found' }, { status: 200 });
    }

    // Update the expert run record
    const artifactFiles = artifacts?.files || [];
    const safeTokens = Math.round(Number.isFinite(tokensUsed) ? tokensUsed : 0);
    const safeDuration = Math.round(Number.isFinite(durationMs) ? durationMs : 0);

    try {
      await db.update(expertRuns).set({
        status: status || 'completed',
        responseText: responseText || null,
        tokensUsed: safeTokens,
        durationMs: safeDuration,
        artifactCount: artifactFiles.length,
        errorMessage: errorMessage || null,
        completedAt: new Date(),
        metadata: { model, engine, engineRunId: runId, artifactDir: artifacts?.artifactDir, frontendRunId },
      }).where(eq(expertRuns.id, dbRunId));
    } catch (updateErr) {
      console.error('[experts/run/complete] expertRuns update failed:', updateErr);
    }

    // Register artifacts as asset records
    if (status === 'completed' && artifactFiles.length > 0) {
      const expertRunId = dbRunId;
      for (const f of artifactFiles) {
        const assetId = `asset-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
        try {
          await db.insert(assets).values({
            id: assetId,
            name: f.fileName,
            folder: `/experts/${artifacts.date || new Date().toISOString().slice(0, 10)}/${artifacts.expertSlug || expertName}`,
            mimeType: f.mimeType || 'application/octet-stream',
            fileType: f.fileType || 'file',
            filePath: f.filePath,
            fileName: f.fileName,
            sizeBytes: f.sizeBytes || 0,
            tags: [f.category || 'response', 'expert-run'],
            metadata: { model, engine, tokensUsed, durationMs, expertName, category: f.category },
            expertId,
            expertRunId,
            sourceType: 'expert',
          });

          // Create lineage
          await db.insert(lineage).values({
            id: `lin-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
            sourceType: 'expert',
            sourceId: expertId,
            targetType: 'asset',
            targetId: assetId,
            relationship: 'produces',
            metadata: { expertRunId, fileName: f.fileName },
          }).catch(() => {});
        } catch (assetErr) {
          console.warn('[experts/run/complete] asset insert failed:', assetErr);
        }
      }
    }

    // Update linked task in task queue
    try {
      const allTasks = await db.select().from(tasks)
        .where(eq(tasks.expertRunId, dbRunId))
        .limit(1);
      if (allTasks.length > 0) {
        await db.update(tasks).set({
          status: status === 'completed' ? 'completed' : 'failed',
          progress: 100,
          currentStep: 1,
          tokensUsed: safeTokens,
          output: responseText?.substring(0, 500) || null,
          errorMessage: errorMessage || null,
          completedAt: new Date(),
          updatedAt: new Date(),
        }).where(eq(tasks.id, allTasks[0].id));
      }
    } catch (taskErr) {
      console.warn('[experts/run/complete] task update failed:', taskErr);
    }

    // Update expert stats — always return to 'idle' (run status is on expertRuns, not expert)
    try {
      const [expert] = await db.select().from(experts).where(eq(experts.id, expertId)).limit(1);
      if (expert) {
        const prevRuns = expert.totalRuns || 0;
        const prevLatency = expert.avgLatencyMs || 0;
        const newAvgLatency = prevRuns > 0
          ? Math.round((prevLatency * prevRuns + safeDuration) / (prevRuns + 1))
          : safeDuration;
        await db.update(experts).set({
          totalRuns: prevRuns + 1,
          avgLatencyMs: newAvgLatency,
          status: 'idle',
          updatedAt: new Date(),
        }).where(eq(experts.id, expertId));
      }
    } catch (statsErr) {
      console.warn('[experts/run/complete] stats update failed:', statsErr);
    }

    logStatus('info', `Expert run ${status}: ${expertName}`, 'expert', {
      runId: dbRunId, expertId, tokensUsed, durationMs, artifactCount: artifactFiles.length,
    });

    return NextResponse.json({ ok: true, runId: dbRunId });
  } catch (err) {
    console.error('[experts/run/complete POST]', err);
    return NextResponse.json({ error: 'Callback processing failed' }, { status: 500 });
  }
}

export const dynamic = 'force-dynamic';
