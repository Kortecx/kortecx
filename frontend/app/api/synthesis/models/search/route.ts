import { NextRequest, NextResponse } from 'next/server';
import { db, apiKeys } from '@/lib/db';
import { sql } from 'drizzle-orm';
import { decryptToken } from '@/lib/oauth/crypto';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* GET /api/synthesis/models/search?q=<query>&source=<ollama|huggingface|llamacpp>&gen_type=<text|image|audio>&limit=10 */
export async function GET(req: NextRequest) {
  const { searchParams } = req.nextUrl;
  const q = searchParams.get('q') ?? '';
  const source = searchParams.get('source') ?? 'ollama';
  const genType = searchParams.get('gen_type') ?? 'text';
  const limit = searchParams.get('limit') ?? '10';

  try {
    const params = new URLSearchParams({ query: q, source, gen_type: genType, limit });
    const headers: Record<string, string> = {};

    // Pass HF token if searching HuggingFace
    if (source === 'huggingface') {
      try {
        const [key] = await db
          .select({ encryptedKey: apiKeys.encryptedKey })
          .from(apiKeys)
          .where(sql`${apiKeys.providerId} = 'huggingface' AND ${apiKeys.status} = 'active'`)
          .limit(1);
        if (key) {
          headers['x-hf-token'] = decryptToken(key.encryptedKey);
        }
      } catch { /* ignore */ }
    }

    const res = await fetch(`${ENGINE_URL}/api/synthesis/models/search?${params}`, { headers });
    if (!res.ok) return NextResponse.json({ models: [], source, query: q });
    return NextResponse.json(await res.json());
  } catch {
    return NextResponse.json({ models: [], source, query: q });
  }
}
