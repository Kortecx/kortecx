import { NextRequest, NextResponse } from 'next/server';
import { db, datasets, synthesisJobs, lineage } from '@/lib/db';
import { eq, desc } from 'drizzle-orm';
import { logStatus } from '@/lib/status-log';

/* GET /api/data/datasets — list all local datasets, auto-sync completed synthesis jobs */
export async function GET() {
  try {
    // Auto-create dataset records for completed synthesis jobs that don't have one yet
    // Match by sourceJobId (not name) to avoid re-creating intentionally deleted datasets
    try {
      const completedJobs = await db.select().from(synthesisJobs)
        .where(eq(synthesisJobs.status, 'completed'));

      if (completedJobs.length > 0) {
        const existingJobIds = new Set(
          (await db.select({ sourceJobId: datasets.sourceJobId }).from(datasets))
            .map((d: { sourceJobId: string | null }) => d.sourceJobId)
            .filter(Boolean)
        );

        for (const job of completedJobs) {
          if (!existingJobIds.has(job.id)) {
            const dsId = `ds-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
            await db.insert(datasets).values({
              id: dsId,
              name: job.name,
              description: job.description ?? `Synthesized with ${job.model} (${job.source})`,
              status: 'ready',
              format: job.outputFormat ?? 'jsonl',
              sampleCount: job.currentSamples ?? job.targetSamples ?? 0,
              sizeBytes: 0,
              qualityScore: null,
              outputPath: job.outputPath ?? null,
              sourceJobId: job.id,
              tags: job.tags ?? [],
              categories: [],
            });

            // Create lineage: synthesis_job → dataset
            try {
              await db.insert(lineage).values({
                id: `lin-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
                sourceType: 'synthesis_job',
                sourceId: job.id,
                targetType: 'dataset',
                targetId: dsId,
                relationship: 'created_by',
                metadata: { model: job.model, source: job.source },
              });
            } catch {}
          }
        }
      }
    } catch (syncErr) {
      console.error('[data/datasets] synthesis sync error:', syncErr);
    }

    const rows = await db.select().from(datasets).orderBy(desc(datasets.updatedAt));
    return NextResponse.json({ datasets: rows, total: rows.length });
  } catch (err) {
    console.error('[data/datasets GET]', err);
    return NextResponse.json({ datasets: [], total: 0 });
  }
}

/* POST /api/data/datasets — create a local dataset record */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { name, description, format, tags, categories, status, sampleCount, sizeBytes, qualityScore } = body;

    if (!name?.trim()) {
      return NextResponse.json({ error: 'Name is required' }, { status: 400 });
    }

    const id = `ds-${Date.now()}`;
    const [inserted] = await db.insert(datasets).values({
      id,
      name: name.trim(),
      description: description?.trim() ?? null,
      status: status ?? 'draft',
      format: format ?? 'jsonl',
      sampleCount: sampleCount ?? 0,
      sizeBytes: sizeBytes ?? 0,
      qualityScore: qualityScore ?? null,
      tags: tags ?? [],
      categories: categories ?? [],
    }).returning();

    logStatus('info', `Dataset created: ${name}`, 'dataset', { datasetId: id, format });

    return NextResponse.json({ dataset: inserted }, { status: 201 });
  } catch (err) {
    console.error('[data/datasets POST]', err);
    return NextResponse.json({ error: 'Failed to create dataset' }, { status: 500 });
  }
}

/* PATCH /api/data/datasets — update a dataset */
export async function PATCH(req: NextRequest) {
  try {
    const body = await req.json();
    const { id, ...updates } = body;
    if (!id) return NextResponse.json({ error: 'id required' }, { status: 400 });

    const values: Record<string, unknown> = { updatedAt: new Date() };
    if (updates.name !== undefined) values.name = updates.name;
    if (updates.description !== undefined) values.description = updates.description;
    if (updates.status !== undefined) values.status = updates.status;
    if (updates.format !== undefined) values.format = updates.format;
    if (updates.sampleCount !== undefined) values.sampleCount = updates.sampleCount;
    if (updates.sizeBytes !== undefined) values.sizeBytes = updates.sizeBytes;
    if (updates.qualityScore !== undefined) values.qualityScore = updates.qualityScore;
    if (updates.tags !== undefined) values.tags = updates.tags;
    if (updates.categories !== undefined) values.categories = updates.categories;

    const [updated] = await db.update(datasets).set(values).where(eq(datasets.id, id)).returning();
    if (!updated) return NextResponse.json({ error: 'Not found' }, { status: 404 });
    logStatus('info', `Dataset updated: ${id}`, 'dataset', { datasetId: id, fields: Object.keys(updates) });
    return NextResponse.json({ dataset: updated });
  } catch (err) {
    console.error('[data/datasets PATCH]', err);
    return NextResponse.json({ error: 'Update failed' }, { status: 500 });
  }
}

/* DELETE /api/data/datasets?id=<id> — also removes linked synthesis job to prevent auto-sync revival */
export async function DELETE(req: NextRequest) {
  try {
    const id = req.nextUrl.searchParams.get('id');
    if (!id) return NextResponse.json({ error: 'id required' }, { status: 400 });
    const [deleted] = await db.delete(datasets).where(eq(datasets.id, id)).returning();
    if (!deleted) return NextResponse.json({ error: 'Not found' }, { status: 404 });

    // Remove the linked synthesis job so the auto-sync in GET won't re-create this dataset
    if (deleted.sourceJobId) {
      await db.delete(synthesisJobs).where(eq(synthesisJobs.id, deleted.sourceJobId)).catch(() => {});
    }

    logStatus('info', `Dataset deleted: ${id}`, 'dataset', { datasetId: id, sourceJobId: deleted.sourceJobId });
    return NextResponse.json({ deleted: true, id });
  } catch (err) {
    console.error('[data/datasets DELETE]', err);
    return NextResponse.json({ error: 'Delete failed' }, { status: 500 });
  }
}
