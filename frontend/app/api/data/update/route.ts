import { NextRequest, NextResponse } from 'next/server';
import { db, dataVersions, lineage } from '@/lib/db';
import { eq } from 'drizzle-orm';
import { logStatus } from '@/lib/status-log';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* POST /api/data/update — update rows in a data file, track version + lineage in NeonDB */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { path, updates, create_version, datasetId } = body;

    if (!path || !updates?.length) {
      return NextResponse.json({ error: 'path and updates required', updated: 0 });
    }

    // Forward to engine for actual file modification
    const res = await fetch(`${ENGINE_URL}/api/data/file/update`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path, updates, create_version: create_version ?? true }),
    });
    const data = await res.json();

    if (data.error) {
      return NextResponse.json(data);
    }

    // Track version in NeonDB if version was created
    if (data.versionPath && datasetId) {
      try {
        // Count existing versions
        const existing = await db.select().from(dataVersions)
          .where(eq(dataVersions.datasetId, datasetId));
        const versionNum = existing.length + 1;

        const versionId = `ver-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
        await db.insert(dataVersions).values({
          id: versionId,
          datasetId,
          versionNum,
          filePath: data.versionPath,
          sizeBytes: 0,
          rowsAffected: data.updated ?? 0,
          changeType: 'edit',
          changeSummary: `Edited ${data.updated} row(s) in ${updates.length} cell(s)`,
          createdBy: 'user',
        });

        // Create lineage record
        await db.insert(lineage).values({
          id: `lin-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
          sourceType: 'dataset',
          sourceId: datasetId,
          targetType: 'dataset',
          targetId: datasetId,
          relationship: 'edited',
          metadata: {
            versionId,
            versionNum,
            cellsChanged: updates.length,
            rowsAffected: data.updated,
          },
        });
      } catch (dbErr) {
        console.error('[data/update] DB tracking error:', dbErr);
      }
    }

    logStatus('info', `Dataset data updated: ${path}`, 'transform', { datasetId, updatesCount: updates.length, versionCreated: create_version });

    return NextResponse.json(data);
  } catch (err) {
    console.error('[data/update POST]', err);
    return NextResponse.json({ error: 'Update failed', updated: 0 });
  }
}
