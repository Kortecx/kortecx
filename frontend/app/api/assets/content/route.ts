import { NextRequest, NextResponse } from 'next/server';
import { db, assets } from '@/lib/db';
import { eq } from 'drizzle-orm';
import { readFile, writeFile } from 'fs/promises';

/* GET /api/assets/content?id=<id> — read text content of an asset file */
export async function GET(req: NextRequest) {
  const id = req.nextUrl.searchParams.get('id');
  if (!id) return NextResponse.json({ error: 'id required' }, { status: 400 });

  try {
    const [asset] = await db.select().from(assets).where(eq(assets.id, id)).limit(1);
    if (!asset) return NextResponse.json({ error: 'Not found' }, { status: 404 });

    const textExts = ['.md', '.txt', '.json', '.py', '.sh', '.js', '.ts', '.yaml', '.yml', '.toml', '.sql', '.html', '.css', '.csv', '.xml', '.log'];
    const ext = '.' + (asset.fileName || '').split('.').pop()?.toLowerCase();

    if (!textExts.includes(ext)) {
      return NextResponse.json({ error: 'Binary file — download instead', filePath: asset.filePath }, { status: 415 });
    }

    const content = await readFile(asset.filePath, 'utf-8');
    return NextResponse.json({ content, fileName: asset.fileName, filePath: asset.filePath });
  } catch (err) {
    console.error('[assets/content GET]', err);
    return NextResponse.json({ error: 'Failed to read file' }, { status: 500 });
  }
}

/* PUT /api/assets/content — save edited content back to disk */
export async function PUT(req: NextRequest) {
  try {
    const body = await req.json();
    const { id, content } = body;
    if (!id || content === undefined) {
      return NextResponse.json({ error: 'id and content required' }, { status: 400 });
    }

    const [asset] = await db.select().from(assets).where(eq(assets.id, id)).limit(1);
    if (!asset) return NextResponse.json({ error: 'Not found' }, { status: 404 });

    const textExts = ['.md', '.txt', '.json', '.py', '.sh', '.js', '.ts', '.yaml', '.yml', '.toml', '.sql', '.html', '.css', '.csv', '.xml', '.log'];
    const ext = '.' + (asset.fileName || '').split('.').pop()?.toLowerCase();

    if (!textExts.includes(ext)) {
      return NextResponse.json({ error: 'Cannot edit binary files' }, { status: 415 });
    }

    await writeFile(asset.filePath, content, 'utf-8');
    await db.update(assets).set({
      sizeBytes: Buffer.byteLength(content, 'utf-8'),
      updatedAt: new Date(),
    }).where(eq(assets.id, id));

    return NextResponse.json({ ok: true, fileName: asset.fileName, sizeBytes: Buffer.byteLength(content, 'utf-8') });
  } catch (err) {
    console.error('[assets/content PUT]', err);
    return NextResponse.json({ error: 'Failed to save file' }, { status: 500 });
  }
}

export const dynamic = 'force-dynamic';
