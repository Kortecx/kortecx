import { NextRequest, NextResponse } from 'next/server';
import { db, apiKeys } from '@/lib/db';
import { sql } from 'drizzle-orm';
import { decryptToken } from '@/lib/oauth/crypto';

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

/* GET /api/models — list local models from ollama + llamacpp */
export async function GET() {
  try {
    const res = await fetch(`${ENGINE_URL}/api/synthesis/models`);
    if (!res.ok) return NextResponse.json({ ollama: [], llamacpp: [], huggingface: [] });
    return NextResponse.json(await res.json());
  } catch {
    return NextResponse.json({ ollama: [], llamacpp: [], huggingface: [] });
  }
}

/* POST /api/models — actions: search | pull | delete | info */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { action } = body;

    switch (action) {
      case 'search': {
        const params = new URLSearchParams({
          query: body.query || '',
          source: body.source || 'ollama',
          gen_type: body.gen_type || 'text',
          limit: String(body.limit || 20),
        });
        const headers: Record<string, string> = {};
        if (body.source === 'huggingface') {
          const token = await getHfToken();
          if (token) headers['x-hf-token'] = token;
        }
        const res = await fetch(`${ENGINE_URL}/api/synthesis/models/search?${params}`, { headers });
        if (!res.ok) return NextResponse.json({ models: [], source: body.source, query: body.query });
        return NextResponse.json(await res.json());
      }

      case 'pull': {
        const res = await fetch(`${ENGINE_URL}/api/orchestrator/models/pull`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            engine: body.engine || 'ollama',
            model: body.model,
          }),
        });
        return NextResponse.json(await res.json());
      }

      case 'delete': {
        const res = await fetch(`${ENGINE_URL}/api/orchestrator/models/delete`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            engine: body.engine || 'ollama',
            model: body.model,
          }),
        });
        return NextResponse.json(await res.json());
      }

      case 'info': {
        const engine = body.engine || 'ollama';
        const model = encodeURIComponent(body.model);
        const res = await fetch(`${ENGINE_URL}/api/orchestrator/models/${engine}/${model}/info`);
        if (!res.ok) return NextResponse.json({ error: 'Model info unavailable' });
        return NextResponse.json(await res.json());
      }

      default:
        return NextResponse.json({ error: 'Unknown action' }, { status: 400 });
    }
  } catch (err) {
    console.error('[POST /api/models]', err);
    return NextResponse.json({ error: 'Model operation failed' }, { status: 500 });
  }
}
