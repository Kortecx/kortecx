import { NextRequest, NextResponse } from 'next/server';
import { db, expertRuns, experts, assets, lineage } from '@/lib/db';
import { eq } from 'drizzle-orm';
import { logStatus } from '@/lib/status-log';

/* POST /api/experts/run/complete — callback from engine when expert run finishes */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const {
      runId, expertId, expertName, status, responseText,
      tokensUsed, durationMs, model, engine, errorMessage, artifacts,
    } = body;

    if (!runId) {
      return NextResponse.json({ error: 'runId required' }, { status: 400 });
    }

    // Find the DB record — match by engineRunId in metadata or by expertId + running status
    const [existingRun] = await db.select().from(expertRuns)
      .where(eq(expertRuns.expertId, expertId))
      .orderBy(expertRuns.createdAt)
      .limit(1);

    // Find the most recent running run for this expert
    const runs = await db.select().from(expertRuns)
      .where(eq(expertRuns.status, 'running'))
      .limit(50);
    const matchingRun = runs.find((r: { expertId: string }) => r.expertId === expertId);
    const dbRunId = matchingRun?.id;

    if (!dbRunId) {
      console.warn('[experts/run/complete] No matching running run found for expert:', expertId);
      return NextResponse.json({ warning: 'No matching run found' }, { status: 200 });
    }

    // Update the expert run record
    const artifactFiles = artifacts?.files || [];
    await db.update(expertRuns).set({
      status: status || 'completed',
      responseText: responseText || null,
      tokensUsed: tokensUsed || 0,
      durationMs: durationMs || 0,
      artifactCount: artifactFiles.length,
      errorMessage: errorMessage || null,
      completedAt: new Date(),
      metadata: { model, engine, engineRunId: runId, artifactDir: artifacts?.artifactDir },
    }).where(eq(expertRuns.id, dbRunId));

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

    // Update expert stats
    if (status === 'completed') {
      try {
        const [expert] = await db.select().from(experts).where(eq(experts.id, expertId)).limit(1);
        if (expert) {
          const prevRuns = expert.totalRuns || 0;
          const prevLatency = expert.avgLatencyMs || 0;
          const newAvgLatency = prevRuns > 0
            ? Math.round((prevLatency * prevRuns + (durationMs || 0)) / (prevRuns + 1))
            : Math.round(durationMs || 0);
          await db.update(experts).set({
            totalRuns: prevRuns + 1,
            avgLatencyMs: newAvgLatency,
            status: 'active',
            updatedAt: new Date(),
          }).where(eq(experts.id, expertId));
        }
      } catch (statsErr) {
        console.warn('[experts/run/complete] stats update failed:', statsErr);
      }
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
