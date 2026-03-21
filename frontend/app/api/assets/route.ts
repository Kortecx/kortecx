import { NextRequest, NextResponse } from 'next/server';
import { db, assets } from '@/lib/db';
import { eq, desc, sql } from 'drizzle-orm';
import { writeFile, mkdir } from 'fs/promises';
import { join, extname } from 'path';
import { randomUUID } from 'crypto';
import { logStatus } from '@/lib/status-log';

const UPLOAD_DIR = join(process.cwd(), '..', 'uploads', 'assets');

function detectFileType(mime: string, ext: string): string {
  if (mime.startsWith('image/')) return 'image';
  if (mime.startsWith('video/')) return 'video';
  if (mime.startsWith('audio/')) return 'audio';
  if (['.pdf', '.doc', '.docx', '.txt', '.md', '.rtf'].includes(ext)) return 'document';
  if (['.csv', '.jsonl', '.json', '.parquet', '.tsv'].includes(ext)) return 'dataset';
  return 'file';
}

/* GET /api/assets?folder=<path> — list assets, optionally filtered by folder */
export async function GET(req: NextRequest) {
  const folder = req.nextUrl.searchParams.get('folder');
  const search = req.nextUrl.searchParams.get('q');

  try {
    let query = db.select().from(assets).orderBy(desc(assets.updatedAt)).$dynamic();

    if (folder) {
      query = query.where(eq(assets.folder, folder));
    }
    if (search) {
      query = query.where(
        sql`(${assets.name} ILIKE ${'%' + search + '%'} OR ${assets.fileName} ILIKE ${'%' + search + '%'})`
      );
    }

    const rows = await query;

    // Get unique folders
    const folderRows = await db
      .selectDistinct({ folder: assets.folder })
      .from(assets)
      .orderBy(assets.folder);
    const folders = folderRows.map((r: { folder: string | null }) => r.folder).filter(Boolean) as string[];

    return NextResponse.json({ assets: rows, total: rows.length, folders });
  } catch (err) {
    console.error('[assets GET]', err);
    return NextResponse.json({ assets: [], total: 0, folders: [] });
  }
}

/* POST /api/assets — upload file(s) and create asset records */
export async function POST(req: NextRequest) {
  try {
    const formData = await req.formData();
    const files = formData.getAll('files') as File[];
    const folder = (formData.get('folder') as string) || '/';
    const tags = (formData.get('tags') as string)?.split(',').map(t => t.trim()).filter(Boolean) ?? [];
    const description = (formData.get('description') as string) || null;

    if (files.length === 0) {
      return NextResponse.json({ error: 'No files provided' }, { status: 400 });
    }

    // Ensure upload directory exists
    const folderDir = join(UPLOAD_DIR, folder.replace(/^\//, ''));
    await mkdir(folderDir, { recursive: true });

    const created = [];
    for (const file of files) {
      const bytes = Buffer.from(await file.arrayBuffer());
      const ext = extname(file.name).toLowerCase();
      const uniqueName = `${randomUUID()}${ext}`;
      const filePath = join(folderDir, uniqueName);

      // Write file to disk
      await writeFile(filePath, bytes);

      // Create DB record
      const id = `asset-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
      const [inserted] = await db.insert(assets).values({
        id,
        name: file.name.replace(ext, ''),
        description,
        folder,
        mimeType: file.type || 'application/octet-stream',
        fileType: detectFileType(file.type, ext),
        filePath,
        fileName: file.name,
        sizeBytes: bytes.length,
        tags,
        metadata: { originalName: file.name, extension: ext },
      }).returning();

      created.push(inserted);
    }

    for (const a of created) {
      logStatus('info', `Asset uploaded: ${a.fileName}`, 'asset', { id: a.id, fileType: a.fileType, sizeBytes: a.sizeBytes });
    }
    return NextResponse.json({ assets: created, count: created.length }, { status: 201 });
  } catch (err) {
    console.error('[assets POST]', err);
    logStatus('error', `Asset upload failed: ${err instanceof Error ? err.message : 'Unknown'}`, 'asset', { error: err instanceof Error ? err.message : 'Unknown' });
    return NextResponse.json({ error: 'Upload failed' }, { status: 500 });
  }
}

/* PATCH /api/assets — update asset metadata (rename, move folder, tags) */
export async function PATCH(req: NextRequest) {
  try {
    const body = await req.json();
    const { id, ...updates } = body;
    if (!id) return NextResponse.json({ error: 'id required' }, { status: 400 });

    const values: Record<string, unknown> = { updatedAt: new Date() };
    if (updates.name !== undefined) values.name = updates.name;
    if (updates.description !== undefined) values.description = updates.description;
    if (updates.folder !== undefined) values.folder = updates.folder;
    if (updates.tags !== undefined) values.tags = updates.tags;

    const [updated] = await db.update(assets).set(values).where(eq(assets.id, id)).returning();
    if (!updated) return NextResponse.json({ error: 'Not found' }, { status: 404 });
    return NextResponse.json({ asset: updated });
  } catch (err) {
    console.error('[assets PATCH]', err);
    return NextResponse.json({ error: 'Update failed' }, { status: 500 });
  }
}

/* DELETE /api/assets?id=<id> — delete asset record (keeps file on disk for safety) */
export async function DELETE(req: NextRequest) {
  try {
    const id = req.nextUrl.searchParams.get('id');
    if (!id) return NextResponse.json({ error: 'id required' }, { status: 400 });
    const [deleted] = await db.delete(assets).where(eq(assets.id, id)).returning();
    if (!deleted) return NextResponse.json({ error: 'Not found' }, { status: 404 });
    logStatus('info', `Asset deleted: ${id}`, 'asset', { id });
    return NextResponse.json({ deleted: true, id });
  } catch (err) {
    console.error('[assets DELETE]', err);
    logStatus('error', `Asset deletion failed: ${err instanceof Error ? err.message : 'Unknown'}`, 'asset', { error: err instanceof Error ? err.message : 'Unknown' });
    return NextResponse.json({ error: 'Delete failed' }, { status: 500 });
  }
}

/* POST /api/assets/folder — create a virtual folder */
export const dynamic = 'force-dynamic';
