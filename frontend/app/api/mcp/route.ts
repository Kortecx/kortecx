import { NextRequest, NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* GET /api/mcp — list all MCP servers (prebuilt + persisted + cached) */
export async function GET() {
  try {
    const res = await fetch(`${ENGINE_URL}/api/mcp/servers`);
    if (!res.ok) return NextResponse.json({ prebuilt: [], persisted: [], cached: [], total: 0 });
    return NextResponse.json(await res.json());
  } catch {
    return NextResponse.json({ prebuilt: [], persisted: [], cached: [], total: 0 });
  }
}

/* POST /api/mcp — route actions: generate | cache | test | persist | update | delete */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { action } = body;

    switch (action) {
      case 'generate': {
        const res = await fetch(`${ENGINE_URL}/api/mcp/generate`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            prompt: body.prompt,
            description: body.description || '',
            language: body.language || 'python',
            model: body.model || 'llama3.1:8b',
            source: body.source || 'ollama',
            system_prompt: body.system_prompt || '',
            prompt_type: body.prompt_type || 'mcp',
          }),
        });
        return NextResponse.json(await res.json());
      }

      case 'cache': {
        const res = await fetch(`${ENGINE_URL}/api/mcp/cache`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            name: body.name,
            description: body.description,
            language: body.language || 'python',
            code: body.code,
            filename: body.filename,
          }),
        });
        return NextResponse.json(await res.json());
      }

      case 'update': {
        const payload: Record<string, unknown> = {};
        if (body.code !== undefined) payload.code = body.code;
        if (body.description !== undefined) payload.description = body.description;
        if (body.is_public !== undefined) payload.is_public = body.is_public;
        const res = await fetch(`${ENGINE_URL}/api/mcp/cache/${body.scriptId}`, {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload),
        });
        return NextResponse.json(await res.json());
      }

      case 'test': {
        const res = await fetch(`${ENGINE_URL}/api/mcp/test/${body.scriptId}`, {
          method: 'POST',
        });
        return NextResponse.json(await res.json());
      }

      case 'persist': {
        const res = await fetch(`${ENGINE_URL}/api/mcp/persist/${body.scriptId}`, {
          method: 'POST',
        });
        return NextResponse.json(await res.json());
      }

      case 'delete_cached': {
        const res = await fetch(`${ENGINE_URL}/api/mcp/cache/${body.scriptId}`, {
          method: 'DELETE',
        });
        return NextResponse.json(await res.json());
      }

      case 'delete_persisted': {
        const res = await fetch(`${ENGINE_URL}/api/mcp/persisted/${body.scriptId}`, {
          method: 'DELETE',
        });
        return NextResponse.json(await res.json());
      }

      case 'set_max_versions': {
        const res = await fetch(`${ENGINE_URL}/api/mcp/config/max-versions`, {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ max_versions: body.max_versions }),
        });
        return NextResponse.json(await res.json());
      }

      default:
        return NextResponse.json({ error: 'Unknown action' }, { status: 400 });
    }
  } catch (err) {
    console.error('[POST /api/mcp]', err);
    return NextResponse.json({ error: 'MCP operation failed' }, { status: 500 });
  }
}
