import { NextRequest, NextResponse } from 'next/server';
import { db, apiKeys } from '@/lib/db';
import { sql } from 'drizzle-orm';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

async function getHfToken(): Promise<string | null> {
  try {
    const [key] = await db
      .select({ encryptedKey: apiKeys.encryptedKey })
      .from(apiKeys)
      .where(sql`${apiKeys.providerId} = 'huggingface' AND ${apiKeys.status} = 'active'`)
      .limit(1);
    if (!key) return null;
    return Buffer.from(key.encryptedKey, 'base64').toString('utf-8');
  } catch {
    return null;
  }
}

/* GET /api/datasets/hf?q=<query>&sort=<sort>&limit=<limit> */
export async function GET(req: NextRequest) {
  const { searchParams } = req.nextUrl;
  const q = searchParams.get('q') ?? '';
  const sort = searchParams.get('sort') ?? 'downloads';
  const limit = searchParams.get('limit') ?? '20';

  const hfToken = await getHfToken();

  try {
    const params = new URLSearchParams({ query: q, sort, limit });
    const headers: Record<string, string> = {};
    if (hfToken) headers['x-hf-token'] = hfToken;

    const res = await fetch(`${ENGINE_URL}/api/datasets/search?${params}`, { headers });
    const data = await res.json();
    return NextResponse.json(data);
  } catch (err) {
    console.error('[datasets/hf GET]', err);
    return NextResponse.json({ datasets: [], count: 0 });
  }
}
