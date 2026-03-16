import { NextRequest, NextResponse } from 'next/server';
import { db, dataVersions, lineage } from '@/lib/db';
import { eq, desc } from 'drizzle-orm';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* POST /api/data/versions — list versions or restore a version */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();

    // Restore mode
    if (body.restore && body.original_path && body.version_path) {
      const res = await fetch(`${ENGINE_URL}/api/data/file/restore`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      const data = await res.json();

      // Track restore in NeonDB
      if (data.restored && body.datasetId) {
        try {
          await db.insert(lineage).values({
            id: `lin-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
            sourceType: 'dataset',
            sourceId: body.datasetId,
            targetType: 'dataset',
            targetId: body.datasetId,
            relationship: 'restored',
            metadata: { restoredFrom: body.version_path },
          });
        } catch {}
      }

      return NextResponse.json(data);
    }

    // List versions — combine NeonDB records + filesystem versions
    const { path, datasetId } = body;

    // Get DB versions
    let dbVersions: any[] = [];
    if (datasetId) {
      dbVersions = await db.select().from(dataVersions)
        .where(eq(dataVersions.datasetId, datasetId))
        .orderBy(desc(dataVersions.createdAt));
    }

    // Get filesystem versions from engine
    let fsVersions: any[] = [];
    if (path) {
      try {
        const res = await fetch(`${ENGINE_URL}/api/data/file/versions`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ path }),
        });
        const data = await res.json();
        fsVersions = data.versions ?? [];
      } catch {}
    }

    // Merge: FS versions are source of truth, enrich with DB metadata
    const merged = fsVersions.map((fv: any) => {
      const dbMatch = dbVersions.find(dv => dv.filePath === fv.path);
      return {
        ...fv,
        id: dbMatch?.id ?? null,
        versionNum: dbMatch?.versionNum ?? null,
        changeType: dbMatch?.changeType ?? 'unknown',
        changeSummary: dbMatch?.changeSummary ?? null,
        rowsAffected: dbMatch?.rowsAffected ?? null,
      };
    });

    return NextResponse.json({ versions: merged, total: merged.length, path });
  } catch (err) {
    console.error('[data/versions POST]', err);
    return NextResponse.json({ versions: [], error: 'Failed' });
  }
}
