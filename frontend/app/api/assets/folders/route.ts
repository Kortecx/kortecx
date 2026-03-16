import { NextRequest, NextResponse } from 'next/server';
import { db, assets } from '@/lib/db';
import { sql } from 'drizzle-orm';

/* GET /api/assets/folders — list all unique folders */
export async function GET() {
  try {
    const rows = await db
      .selectDistinct({ folder: assets.folder })
      .from(assets)
      .orderBy(assets.folder);
    const folders = ['/', ...rows.map((r: { folder: string | null }) => r.folder).filter((f: string | null): f is string => !!f && f !== '/')];
    return NextResponse.json({ folders });
  } catch {
    return NextResponse.json({ folders: ['/'] });
  }
}

/* POST /api/assets/folders — create a placeholder asset to register a folder */
export async function POST(req: NextRequest) {
  try {
    const { name, parent } = await req.json();
    if (!name?.trim()) return NextResponse.json({ error: 'Folder name required' }, { status: 400 });

    const parentPath = (parent || '/').replace(/\/$/, '');
    const folderPath = parentPath === '/' ? `/${name.trim()}` : `${parentPath}/${name.trim()}`;

    // Check if folder already has assets
    const [existing] = await db
      .select({ id: assets.id })
      .from(assets)
      .where(sql`${assets.folder} = ${folderPath}`)
      .limit(1);

    return NextResponse.json({ folder: folderPath, exists: !!existing });
  } catch (err) {
    console.error('[assets/folders POST]', err);
    return NextResponse.json({ error: 'Failed to create folder' }, { status: 500 });
  }
}
