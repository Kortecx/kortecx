import { NextRequest, NextResponse } from 'next/server';
import { db, synthesisJobs, datasets, apiKeys } from '@/lib/db';
import { eq, desc, sql } from 'drizzle-orm';
import { logStatus } from '@/lib/status-log';
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
      qdrantCollection, tags, categories, schema,
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

    logStatus('info', `Synthesis job started: ${name}`, 'synthesis', { jobId: id, model, source, targetSamples });

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
        qdrantCollection, tags, categories, schema,
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

/* PATCH /api/synthesis — update job name/config, optionally restart */
export async function PATCH(req: NextRequest) {
  try {
    const body = await req.json();
    const { id, restart, ...updates } = body;
    if (!id) return NextResponse.json({ error: 'id required' }, { status: 400 });

    const [existing] = await db.select().from(synthesisJobs).where(eq(synthesisJobs.id, id));
    if (!existing) return NextResponse.json({ error: 'Not found' }, { status: 404 });

    // Build update values
    const values: Record<string, unknown> = {};
    if (updates.name !== undefined)          values.name = updates.name.trim();
    if (updates.description !== undefined)   values.description = updates.description?.trim() ?? null;
    if (updates.source !== undefined)        values.source = updates.source;
    if (updates.model !== undefined)         values.model = updates.model.trim();
    if (updates.targetSamples !== undefined) values.targetSamples = updates.targetSamples;
    if (updates.outputFormat !== undefined)  values.outputFormat = updates.outputFormat;
    if (updates.temperature !== undefined)   values.temperature = String(updates.temperature);
    if (updates.maxTokens !== undefined)     values.maxTokens = updates.maxTokens;
    if (updates.batchSize !== undefined)     values.batchSize = updates.batchSize;
    if (updates.tags !== undefined)          values.tags = updates.tags;
    if (updates.status !== undefined)        values.status = updates.status;

    // If cancelling a running job, tell the engine
    if (updates.status === 'cancelled' && (existing.status === 'running' || existing.status === 'queued')) {
      logStatus('info', `Synthesis cancelled: ${existing.name}`, 'synthesis', { jobId: id });
      const hfToken = await getHfToken();
      const headers: Record<string, string> = { 'Content-Type': 'application/json' };
      if (hfToken) headers['x-hf-token'] = hfToken;
      // Fire-and-forget cancel to engine (best effort)
      fetch(`${ENGINE_URL}/api/synthesis/jobs/${id}/cancel`, { method: 'POST', headers }).catch(() => {});
    }

    const [updated] = await db.update(synthesisJobs).set(values).where(eq(synthesisJobs.id, id)).returning();

    // If restart requested — cancel old job, reset progress, re-start synthesis
    if (restart && updated) {
      // Cancel on engine if still running
      if (existing.status === 'running') {
        const hfToken = await getHfToken();
        const headers: Record<string, string> = { 'Content-Type': 'application/json' };
        if (hfToken) headers['x-hf-token'] = hfToken;
        fetch(`${ENGINE_URL}/api/synthesis/jobs/${id}/cancel`, { method: 'POST', headers }).catch(() => {});
      }

      // Reset progress
      await db.update(synthesisJobs).set({
        status: 'queued',
        currentSamples: 0,
        progress: 0,
        tokensUsed: 0,
        error: null,
        outputPath: null,
        startedAt: null,
        completedAt: null,
      }).where(eq(synthesisJobs.id, id));

      // Re-start on engine
      const hfToken = await getHfToken();
      const headers: Record<string, string> = { 'Content-Type': 'application/json' };
      if (hfToken) headers['x-hf-token'] = hfToken;

      const jobData = { ...existing, ...values };
      fetch(`${ENGINE_URL}/api/synthesis/start`, {
        method: 'POST',
        headers,
        body: JSON.stringify({
          name: jobData.name,
          description: jobData.description,
          source: jobData.source,
          model: jobData.model,
          targetSamples: jobData.targetSamples,
          outputFormat: jobData.outputFormat,
          temperature: Number(jobData.temperature),
          maxTokens: jobData.maxTokens,
          batchSize: jobData.batchSize,
          tags: jobData.tags,
        }),
      })
        .then(async (res) => {
          if (res.ok) {
            const data = await res.json();
            await db.update(synthesisJobs)
              .set({ status: 'running', startedAt: new Date() })
              .where(eq(synthesisJobs.id, id));
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

      return NextResponse.json({ job: updated, restarted: true });
    }

    return NextResponse.json({ job: updated });
  } catch (err) {
    console.error('[synthesis PATCH]', err);
    return NextResponse.json({ error: 'Update failed' }, { status: 500 });
  }
}

/* DELETE /api/synthesis?id=<id> — remove a synthesis job and its linked dataset */
export async function DELETE(req: NextRequest) {
  try {
    const id = req.nextUrl.searchParams.get('id');
    if (!id) return NextResponse.json({ error: 'id required' }, { status: 400 });
    const [deleted] = await db.delete(synthesisJobs).where(eq(synthesisJobs.id, id)).returning();
    if (!deleted) return NextResponse.json({ error: 'Not found' }, { status: 404 });

    // Also remove the dataset that was created from this job
    await db.delete(datasets).where(eq(datasets.sourceJobId, id)).catch(() => {});

    logStatus('info', `Synthesis job removed: ${id}`, 'synthesis', { jobId: id });
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

        // Auto-create a dataset record from the completed synthesis job
        try {
          const [job] = await db.select().from(synthesisJobs).where(eq(synthesisJobs.id, dbId));
          if (job) {
            const dsId = `ds-${Date.now()}`;
            await db.insert(datasets).values({
              id: dsId,
              name: job.name,
              description: job.description ?? `Synthesized using ${job.model} (${job.source})`,
              status: 'ready',
              format: job.outputFormat ?? 'jsonl',
              sampleCount: data.currentSamples ?? job.targetSamples ?? 0,
              sizeBytes: 0,
              qualityScore: null,
              outputPath: data.outputPath ?? null,
              sourceJobId: dbId,
              tags: job.tags ?? [],
              categories: [],
            }).onConflictDoNothing();
            logStatus('info', `Synthesis completed: ${job.name} — ${data.currentSamples} samples`, 'synthesis', { jobId: dbId, datasetId: dsId, samples: data.currentSamples });
          }
        } catch (err) {
          console.error('[synthesis] Failed to create dataset record:', err);
        }
      } else if (data.status === 'failed') {
        logStatus('error', `Synthesis failed: ${dbId}`, 'synthesis', { jobId: dbId, error: data.error });
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
