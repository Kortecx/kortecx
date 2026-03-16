import { NextRequest, NextResponse } from 'next/server';
import { db, synthesisJobs, apiKeys } from '@/lib/db';
import { eq, desc, sql } from 'drizzle-orm';

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

/* GET /api/synthesis — list all synthesis jobs */
export async function GET() {
  try {
    const rows = await db.select().from(synthesisJobs).orderBy(desc(synthesisJobs.createdAt));
    return NextResponse.json({ jobs: rows, total: rows.length });
  } catch (err) {
    console.error('[synthesis GET]', err);
    return NextResponse.json({ jobs: [], total: 0 });
  }
}

/* POST /api/synthesis — create and start a synthesis job */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const {
      name, description, source, model, baseUrl,
      promptTemplate, systemPrompt, targetSamples, outputFormat,
      temperature, maxTokens, batchSize, saveToQdrant,
      qdrantCollection, tags, categories,
    } = body;

    if (!name?.trim()) {
      return NextResponse.json({ error: 'Name is required' }, { status: 400 });
    }
    if (!model?.trim()) {
      return NextResponse.json({ error: 'Model is required' }, { status: 400 });
    }

    // Insert job record in DB
    const id = `synth-${Date.now()}`;
    const [inserted] = await db.insert(synthesisJobs).values({
      id,
      name: name.trim(),
      description: description?.trim() ?? null,
      source: source ?? 'ollama',
      model: model.trim(),
      status: 'queued',
      targetSamples: targetSamples ?? 100,
      outputFormat: outputFormat ?? 'jsonl',
      temperature: String(temperature ?? 0.8),
      maxTokens: maxTokens ?? 1024,
      batchSize: batchSize ?? 5,
      tags: tags ?? [],
    }).returning();

    // Forward to engine to start generation
    const hfToken = await getHfToken();
    const headers: Record<string, string> = { 'Content-Type': 'application/json' };
    if (hfToken) headers['x-hf-token'] = hfToken;

    fetch(`${ENGINE_URL}/api/synthesis/start`, {
      method: 'POST',
      headers,
      body: JSON.stringify({
        name, description, source, model, baseUrl,
        promptTemplate, systemPrompt, targetSamples, outputFormat,
        temperature, maxTokens, batchSize, saveToQdrant,
        qdrantCollection, tags, categories,
      }),
    })
      .then(async (res) => {
        if (res.ok) {
          const data = await res.json();
          await db.update(synthesisJobs)
            .set({ status: 'running', startedAt: new Date() })
            .where(eq(synthesisJobs.id, id));

          // Poll for completion
          pollSynthesisJob(id, data.jobId, headers);
        } else {
          const err = await res.text();
          await db.update(synthesisJobs)
            .set({ status: 'failed', error: err })
            .where(eq(synthesisJobs.id, id));
        }
      })
      .catch(async (err) => {
        await db.update(synthesisJobs)
          .set({ status: 'failed', error: String(err) })
          .where(eq(synthesisJobs.id, id));
      });

    return NextResponse.json({ job: inserted, message: 'Synthesis started' }, { status: 201 });
  } catch (err) {
    console.error('[synthesis POST]', err);
    return NextResponse.json({ error: 'Failed to start synthesis' }, { status: 500 });
  }
}

/* DELETE /api/synthesis?id=<id> — remove a synthesis job record */
export async function DELETE(req: NextRequest) {
  try {
    const id = req.nextUrl.searchParams.get('id');
    if (!id) return NextResponse.json({ error: 'id required' }, { status: 400 });
    const [deleted] = await db.delete(synthesisJobs).where(eq(synthesisJobs.id, id)).returning();
    if (!deleted) return NextResponse.json({ error: 'Not found' }, { status: 404 });
    return NextResponse.json({ deleted: true, id });
  } catch (err) {
    console.error('[synthesis DELETE]', err);
    return NextResponse.json({ error: 'Delete failed' }, { status: 500 });
  }
}

/** Poll engine for job status and update DB */
function pollSynthesisJob(dbId: string, engineJobId: string, headers: Record<string, string>) {
  const interval = setInterval(async () => {
    try {
      const res = await fetch(`${ENGINE_URL}/api/synthesis/jobs/${engineJobId}`, { headers });
      if (!res.ok) return;
      const data = await res.json();

      const updates: Record<string, unknown> = {
        currentSamples: data.currentSamples ?? 0,
        progress: data.progress ?? 0,
        tokensUsed: data.tokensUsed ?? 0,
      };

      if (data.status === 'completed') {
        updates.status = 'completed';
        updates.outputPath = data.outputPath ?? null;
        updates.completedAt = new Date();
        clearInterval(interval);
      } else if (data.status === 'failed') {
        updates.status = 'failed';
        updates.error = data.error ?? 'Unknown error';
        clearInterval(interval);
      } else if (data.status === 'cancelled') {
        updates.status = 'cancelled';
        clearInterval(interval);
      } else {
        updates.status = 'running';
      }

      await db.update(synthesisJobs).set(updates).where(eq(synthesisJobs.id, dbId));
    } catch {
      // Silently retry on next interval
    }
  }, 3000);

  // Safety: stop polling after 30 minutes
  setTimeout(() => clearInterval(interval), 30 * 60 * 1000);
}
