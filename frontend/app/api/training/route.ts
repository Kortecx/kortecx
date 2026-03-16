import { NextRequest, NextResponse } from 'next/server';
import { db, trainingJobs, datasets } from '@/lib/db';
import { desc, eq } from 'drizzle-orm';

/* GET /api/training */
export async function GET() {
  try {
    const jobRows = await db.select().from(trainingJobs).orderBy(desc(trainingJobs.createdAt));
    const datasetRows = await db.select().from(datasets).orderBy(desc(datasets.createdAt));

    let activeCount = 0;
    let queuedCount = 0;
    for (const j of jobRows) {
      if (j.status === 'training') activeCount++;
      if (j.status === 'queued') queuedCount++;
    }

    return NextResponse.json({
      jobs: jobRows,
      datasets: datasetRows,
      activeCount,
      queuedCount,
    });
  } catch {
    return NextResponse.json({ jobs: [], datasets: [], activeCount: 0, queuedCount: 0 });
  }
}

/* POST /api/training — Start a training job */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const {
      name, expertId, baseModelId, datasetId,
      epochs = 5, learningRate = 2e-5, batchSize = 16,
    } = body;

    if (!name || !baseModelId || !datasetId) {
      return NextResponse.json(
        { error: 'name, baseModelId, datasetId are required' },
        { status: 400 },
      );
    }

    const [dsRow] = await db.select().from(datasets).where(eq(datasets.id, datasetId));
    if (!dsRow || dsRow.status !== 'ready') {
      return NextResponse.json({ error: `Dataset ${datasetId} is not ready` }, { status: 409 });
    }

    const trainingSamples = Math.floor((dsRow.sampleCount ?? 0) * 0.9);
    const validationSamples = (dsRow.sampleCount ?? 0) - trainingSamples;
    const id = `job-${Date.now()}`;

    const [inserted] = await db.insert(trainingJobs).values({
      id,
      name,
      expertId: expertId ?? null,
      baseModelId,
      datasetId,
      status: 'queued',
      progress: 0,
      epochs,
      learningRate: String(learningRate),
      batchSize,
      trainingSamples,
      validationSamples,
    }).returning();

    return NextResponse.json({ job: inserted, message: `Training job "${name}" queued` }, { status: 201 });
  } catch (err) {
    const message = err instanceof Error ? err.message : 'Invalid request body';
    return NextResponse.json({ error: message }, { status: 400 });
  }
}
