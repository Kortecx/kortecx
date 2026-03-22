import { NextRequest, NextResponse } from 'next/server';
import { db, assets, lineage } from '@/lib/db';
import { logStatus } from '@/lib/status-log';

interface AssetRecord {
  name: string;
  fileName: string;
  filePath: string;
  sizeBytes: number;
  mimeType: string;
  fileType: string;
  expertId?: string;
  expertRunId?: string;
  sourceType?: string;
  folder?: string;
  tags?: string[];
  metadata?: Record<string, unknown>;
  description?: string;
}

/* POST /api/assets/register — register pre-existing files as asset records (no upload) */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const records: AssetRecord[] = body.assets;

    if (!records || records.length === 0) {
      return NextResponse.json({ error: 'No asset records provided' }, { status: 400 });
    }

    const created = [];
    for (const record of records) {
      const id = `asset-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

      const [inserted] = await db.insert(assets).values({
        id,
        name: record.name || record.fileName,
        description: record.description || null,
        folder: record.folder || '/experts',
        mimeType: record.mimeType || 'application/octet-stream',
        fileType: record.fileType || 'file',
        filePath: record.filePath,
        fileName: record.fileName,
        sizeBytes: record.sizeBytes || 0,
        tags: record.tags || [],
        metadata: record.metadata || {},
        expertId: record.expertId || null,
        expertRunId: record.expertRunId || null,
        sourceType: record.sourceType || 'expert',
        datasetId: null,
      }).returning();

      created.push(inserted);

      // Create lineage record: expert -> asset
      if (record.expertId) {
        const lineageId = `lin-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
        await db.insert(lineage).values({
          id: lineageId,
          sourceType: 'expert',
          sourceId: record.expertId,
          targetType: 'asset',
          targetId: id,
          relationship: 'produces',
          metadata: {
            expertRunId: record.expertRunId,
            fileName: record.fileName,
            fileType: record.fileType,
          },
        }).catch((e: unknown) => {
          console.warn('[assets/register] lineage insert failed:', e);
        });
      }
    }

    logStatus('info', `Registered ${created.length} expert assets`, 'asset', {
      count: created.length,
      expertId: records[0]?.expertId,
      sourceType: records[0]?.sourceType,
    });

    return NextResponse.json({ assets: created, count: created.length }, { status: 201 });
  } catch (err) {
    console.error('[assets/register POST]', err);
    return NextResponse.json({ error: 'Registration failed' }, { status: 500 });
  }
}

export const dynamic = 'force-dynamic';
