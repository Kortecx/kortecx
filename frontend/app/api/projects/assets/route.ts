import { NextRequest, NextResponse } from 'next/server';
import { db, projectAssets } from '@/lib/db';
import { eq } from 'drizzle-orm';
import { logStatus } from '@/lib/status-log';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* GET /api/projects/assets?projectId=<id> — list assets in a project */
export async function GET(req: NextRequest) {
  try {
    const projectId = req.nextUrl.searchParams.get('projectId');
    if (!projectId) return NextResponse.json({ error: 'projectId required' }, { status: 400 });

    const rows = await db.select().from(projectAssets)
      .where(eq(projectAssets.projectId, projectId));
    return NextResponse.json({ assets: rows, total: rows.length });
  } catch (err) {
    console.error('[projects/assets GET]', err);
    return NextResponse.json({ assets: [], total: 0 });
  }
}

/* POST /api/projects/assets — add an asset to a project */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { projectId, assetType, assetId, assetName, assetPath, mlflowRunId, metadata } = body;

    if (!projectId || !assetType || !assetId || !assetName) {
      return NextResponse.json({ error: 'projectId, assetType, assetId, and assetName are required' }, { status: 400 });
    }

    const id = `pa-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
    const [inserted] = await db.insert(projectAssets).values({
      id,
      projectId,
      assetType,
      assetId,
      assetName,
      assetPath: assetPath ?? null,
      mlflowRunId: mlflowRunId ?? null,
      metadata: metadata ?? null,
    }).returning();

    // Log to MLflow if engine is available
    if (assetPath) {
      fetch(`${ENGINE_URL}/api/mlflow/log/asset`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: assetName,
          path: assetPath,
          assetType,
          project: projectId,
          tags: { assetId, projectId },
        }),
      }).catch(() => {});
    }

    logStatus('info', `Asset added to project: ${assetName}`, 'project', { projectId, assetType, assetId });
    return NextResponse.json({ asset: inserted }, { status: 201 });
  } catch (err) {
    console.error('[projects/assets POST]', err);
    return NextResponse.json({ error: 'Failed to add asset' }, { status: 500 });
  }
}

/* DELETE /api/projects/assets?id=<id> — remove an asset from a project */
export async function DELETE(req: NextRequest) {
  try {
    const id = req.nextUrl.searchParams.get('id');
    if (!id) return NextResponse.json({ error: 'id required' }, { status: 400 });
    const [deleted] = await db.delete(projectAssets).where(eq(projectAssets.id, id)).returning();
    if (!deleted) return NextResponse.json({ error: 'Not found' }, { status: 404 });
    logStatus('info', `Asset removed from project: ${deleted.assetName}`, 'project', { id, projectId: deleted.projectId });
    return NextResponse.json({ deleted: true, id });
  } catch (err) {
    console.error('[projects/assets DELETE]', err);
    return NextResponse.json({ error: 'Delete failed' }, { status: 500 });
  }
}
