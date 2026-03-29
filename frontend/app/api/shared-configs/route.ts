import { NextResponse } from 'next/server';
import { readdir, readFile, stat } from 'fs/promises';
import path from 'path';

const SHARED_DIR = process.env.KORTECX_SHARED_CONFIG_DIR || path.join(process.cwd(), 'shared_configs');

/* GET /api/shared-configs — list available config files from shared directory */
export async function GET() {
  try {
    // Ensure directory exists
    let files: string[];
    try {
      files = await readdir(SHARED_DIR);
    } catch {
      return NextResponse.json({ configs: [], directory: SHARED_DIR, exists: false });
    }

    const jsonFiles = files.filter(f => f.endsWith('.json'));

    const configs = await Promise.all(
      jsonFiles.map(async (filename) => {
        const filePath = path.join(SHARED_DIR, filename);
        try {
          const stats = await stat(filePath);
          // Read first portion to extract metadata
          const content = await readFile(filePath, 'utf-8');
          const parsed = JSON.parse(content);

          if (parsed._kortecxExport !== true) return null;

          // Determine a display name from the entity data
          let name = filename.replace('.json', '');
          const entityType = parsed._entityType || 'unknown';
          if (parsed.expert?.name) name = parsed.expert.name;
          else if (parsed.workflow?.name) name = parsed.workflow.name;
          else if (parsed.dataset?.name) name = parsed.dataset.name;
          else if (parsed.server?.name) name = parsed.server.name;

          return {
            filename,
            entityType,
            name,
            exportedAt: parsed._exportedAt || null,
            version: parsed._version || null,
            sizeBytes: stats.size,
          };
        } catch {
          return null;
        }
      }),
    );

    return NextResponse.json({
      configs: configs.filter(Boolean),
      directory: SHARED_DIR,
      exists: true,
    });
  } catch (err) {
    console.error('Shared configs error:', err);
    return NextResponse.json({ error: 'Failed to list shared configs' }, { status: 500 });
  }
}
