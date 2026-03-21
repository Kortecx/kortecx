import { NextRequest, NextResponse } from 'next/server';
import { db, hfDatasets, apiKeys } from '@/lib/db';
import { eq, desc, sql } from 'drizzle-orm';
import { decryptToken } from '@/lib/oauth/crypto';
import { logStatus } from '@/lib/status-log';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

async function getHfToken(): Promise<string | null> {
  try {
    const [key] = await db
      .select({ encryptedKey: apiKeys.encryptedKey })
      .from(apiKeys)
      .where(sql`${apiKeys.providerId} = 'huggingface' AND ${apiKeys.status} = 'active'`)
      .limit(1);
    if (!key) return null;
    return decryptToken(key.encryptedKey);
  } catch {
    return null;
  }
}

/* GET /api/datasets — list tracked HF datasets or get one by id */
export async function GET(req: NextRequest) {
  const { searchParams } = req.nextUrl;
  const id = searchParams.get('id');

  try {
    if (id) {
      const [row] = await db.select().from(hfDatasets).where(eq(hfDatasets.id, id));
      if (!row) return NextResponse.json({ error: 'Dataset not found' }, { status: 404 });
      return NextResponse.json({ dataset: row });
    }

    const rows = await db.select().from(hfDatasets).orderBy(desc(hfDatasets.updatedAt));
    return NextResponse.json({ datasets: rows, total: rows.length });
  } catch (err) {
    console.error('[datasets GET]', err);
    return NextResponse.json({ datasets: [], total: 0 });
  }
}

/* POST /api/datasets — track + download a HF dataset */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { hfId, author, name, description, tags, downloads, likes, config } = body;

    if (!hfId?.trim()) {
      return NextResponse.json({ error: 'hfId is required' }, { status: 400 });
    }

    const id = `hfds-${Date.now()}`;

    // Insert as "downloading" status
    const [inserted] = await db.insert(hfDatasets).values({
      id,
      hfId: hfId.trim(),
      author: author ?? null,
      name: name?.trim() || hfId.split('/').pop() || hfId,
      description: description ?? null,
      tags: tags ?? [],
      downloads: downloads ?? 0,
      likes: likes ?? 0,
      config: config ?? null,
      status: 'downloading',
    }).returning();

    // Trigger download on engine (fire and forget)
    const hfToken = await getHfToken();
    const engineHeaders: Record<string, string> = { 'Content-Type': 'application/json' };
    if (hfToken) engineHeaders['x-hf-token'] = hfToken;

    fetch(`${ENGINE_URL}/api/datasets/download`, {
      method: 'POST',
      headers: engineHeaders,
      body: JSON.stringify({ dataset_id: hfId, config }),
    })
      .then(async (res) => {
        if (res.ok) {
          const data = await res.json();
          await db.update(hfDatasets)
            .set({
              status: 'downloaded',
              splits: data.splits,
              numRows: data.num_rows,
              columns: data.columns,
              features: data.features,
              cachePath: data.cache_path,
              sizeBytes: data.size_bytes,
              downloadedAt: new Date(),
              updatedAt: new Date(),
            })
            .where(eq(hfDatasets.id, id));
        } else {
          const err = await res.text();
          await db.update(hfDatasets)
            .set({ status: 'error', errorMessage: err, updatedAt: new Date() })
            .where(eq(hfDatasets.id, id));
        }
      })
      .catch(async (err) => {
        await db.update(hfDatasets)
          .set({ status: 'error', errorMessage: String(err), updatedAt: new Date() })
          .where(eq(hfDatasets.id, id));
      });

    logStatus('info', `HF dataset tracked: ${name?.trim() || hfId}`, 'dataset', { id, hfId });
    return NextResponse.json({ dataset: inserted, message: 'Download started' }, { status: 201 });
  } catch (err) {
    console.error('[datasets POST]', err);
    logStatus('error', `Dataset creation failed: ${err instanceof Error ? err.message : 'Unknown'}`, 'dataset', { error: err instanceof Error ? err.message : 'Unknown' });
    return NextResponse.json({ error: 'Failed to create dataset' }, { status: 500 });
  }
}

/* PATCH /api/datasets — update tracked dataset */
export async function PATCH(req: NextRequest) {
  try {
    const body = await req.json();
    const { id, ...updates } = body;

    if (!id) return NextResponse.json({ error: 'id is required' }, { status: 400 });

    const values: Record<string, unknown> = { updatedAt: new Date() };
    if (updates.status !== undefined) values.status = updates.status;
    if (updates.errorMessage !== undefined) values.errorMessage = updates.errorMessage;
    if (updates.splits !== undefined) values.splits = updates.splits;
    if (updates.numRows !== undefined) values.numRows = updates.numRows;
    if (updates.columns !== undefined) values.columns = updates.columns;
    if (updates.features !== undefined) values.features = updates.features;
    if (updates.cachePath !== undefined) values.cachePath = updates.cachePath;
    if (updates.sizeBytes !== undefined) values.sizeBytes = updates.sizeBytes;

    const [updated] = await db.update(hfDatasets)
      .set(values)
      .where(eq(hfDatasets.id, id))
      .returning();

    if (!updated) return NextResponse.json({ error: 'Dataset not found' }, { status: 404 });
    return NextResponse.json({ dataset: updated });
  } catch (err) {
    console.error('[datasets PATCH]', err);
    return NextResponse.json({ error: 'Update failed' }, { status: 500 });
  }
}

/* DELETE /api/datasets — remove tracked dataset */
export async function DELETE(req: NextRequest) {
  try {
    const { searchParams } = req.nextUrl;
    const id = searchParams.get('id');
    if (!id) return NextResponse.json({ error: 'id is required' }, { status: 400 });

    const [deleted] = await db.delete(hfDatasets).where(eq(hfDatasets.id, id)).returning();
    if (!deleted) return NextResponse.json({ error: 'Dataset not found' }, { status: 404 });
    logStatus('info', `HF dataset removed: ${id}`, 'dataset', { id });
    return NextResponse.json({ deleted: true, id });
  } catch (err) {
    console.error('[datasets DELETE]', err);
    logStatus('error', `Dataset deletion failed: ${err instanceof Error ? err.message : 'Unknown'}`, 'dataset', { error: err instanceof Error ? err.message : 'Unknown' });
    return NextResponse.json({ error: 'Delete failed' }, { status: 500 });
  }
}
