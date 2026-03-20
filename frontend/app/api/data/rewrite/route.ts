import { NextRequest, NextResponse } from 'next/server';
import { db, dataVersions, lineage } from '@/lib/db';
import { eq } from 'drizzle-orm';
import { logStatus } from '@/lib/status-log';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* POST /api/data/rewrite — overwrite a data file with transformed rows, track version + lineage */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { path, rows, columns, create_version, datasetId } = body;

    if (!path || !rows?.length) {
      return NextResponse.json({ error: 'path and rows required', written: 0 });
    }

    // Forward to engine for actual file rewrite
    const res = await fetch(`${ENGINE_URL}/api/data/file/rewrite`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path, rows, columns, create_version: create_version ?? true }),
    });
    const data = await res.json();

    if (data.error) {
      return NextResponse.json(data);
    }

    // Track version in NeonDB if version was created
    if (data.versionPath && datasetId) {
      try {
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
          rowsAffected: data.written ?? 0,
          changeType: 'edit',
          changeSummary: `Rewritten with ${data.written} rows (transform save)`,
          createdBy: 'user',
        });

        await db.insert(lineage).values({
          id: `lin-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
          sourceType: 'dataset',
          sourceId: datasetId,
          targetType: 'dataset',
          targetId: datasetId,
          relationship: 'edited',
          metadata: { versionId, versionNum, action: 'transform_rewrite', rowsWritten: data.written },
        });
      } catch (dbErr) {
        console.error('[data/rewrite] DB tracking error:', dbErr);
      }
    }

    logStatus('info', `Dataset rewritten: ${path} (${data.written} rows)`, 'transform', { datasetId, rowsWritten: data.written });

    return NextResponse.json(data);
  } catch (err) {
    console.error('[data/rewrite POST]', err);
    return NextResponse.json({ error: 'Rewrite failed', written: 0 });
  }
}
