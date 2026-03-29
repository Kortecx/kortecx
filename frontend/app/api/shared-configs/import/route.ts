import { NextRequest, NextResponse } from 'next/server';
import { readFile } from 'fs/promises';
import path from 'path';

const SHARED_DIR = process.env.KORTECX_SHARED_CONFIG_DIR || path.join(process.cwd(), 'shared_configs');

/* POST /api/shared-configs/import — import a config file from the shared directory */
export async function POST(req: NextRequest) {
  const { filename } = await req.json();

  if (!filename || typeof filename !== 'string') {
    return NextResponse.json({ error: 'Missing filename' }, { status: 400 });
  }

  // Security: prevent path traversal
  const basename = path.basename(filename);
  if (basename !== filename || filename.includes('..') || filename.includes('/') || filename.includes('\\')) {
    return NextResponse.json({ error: 'Invalid filename' }, { status: 400 });
  }

  if (!filename.endsWith('.json')) {
    return NextResponse.json({ error: 'Only .json files are supported' }, { status: 400 });
  }

  const filePath = path.join(SHARED_DIR, basename);

  // Verify the resolved path is still within the shared directory
  const resolved = path.resolve(filePath);
  const resolvedDir = path.resolve(SHARED_DIR);
  if (!resolved.startsWith(resolvedDir + path.sep) && resolved !== resolvedDir) {
    return NextResponse.json({ error: 'Invalid file path' }, { status: 400 });
  }

  try {
    const content = await readFile(filePath, 'utf-8');
    const config = JSON.parse(content);

    if (config._kortecxExport !== true) {
      return NextResponse.json({ error: 'File is not a valid Kortecx export' }, { status: 400 });
    }

    // Delegate to the import API
    const importRes = await fetch(new URL('/api/import', req.url).toString(), {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(config),
    });

    if (!importRes.ok) {
      const err = await importRes.json().catch(() => ({ error: 'Import failed' }));
      return NextResponse.json(err, { status: importRes.status });
    }

    return NextResponse.json(await importRes.json());
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') {
      return NextResponse.json({ error: 'File not found' }, { status: 404 });
    }
    console.error('Shared config import error:', err);
    return NextResponse.json({ error: 'Failed to import config' }, { status: 500 });
  }
}
