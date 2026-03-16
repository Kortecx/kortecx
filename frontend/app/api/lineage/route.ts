import { NextRequest, NextResponse } from 'next/server';
import { db, lineage, datasets, trainingJobs, workflows, workflowSteps, experts, synthesisJobs } from '@/lib/db';
import { eq, or, sql } from 'drizzle-orm';

/* GET /api/lineage?sourceId=<id>&sourceType=<type> — get all lineage for a source */
/* GET /api/lineage?targetId=<id>&targetType=<type> — get all lineage for a target */
/* GET /api/lineage?impact=<datasetId> — get downstream impact analysis for a dataset */
export async function GET(req: NextRequest) {
  const { searchParams } = req.nextUrl;
  const sourceId = searchParams.get('sourceId');
  const sourceType = searchParams.get('sourceType');
  const targetId = searchParams.get('targetId');
  const targetType = searchParams.get('targetType');
  const impactId = searchParams.get('impact');

  try {
    // Impact analysis — find all downstream dependencies of a dataset
    if (impactId) {
      // Find training jobs using this dataset
      const trainingDeps = await db.select({
        id: trainingJobs.id,
        name: trainingJobs.name,
        status: trainingJobs.status,
      }).from(trainingJobs).where(eq(trainingJobs.datasetId, impactId));

      // Find experts trained on this dataset (via training jobs)
      const trainedExperts: any[] = [];
      for (const job of trainingDeps) {
        if (job.id) {
          const exps = await db.select({
            id: experts.id,
            name: experts.name,
            status: experts.status,
          }).from(experts).where(sql`${experts.id} IN (
            SELECT expert_id FROM training_jobs WHERE dataset_id = ${impactId} AND expert_id IS NOT NULL
          )`);
          trainedExperts.push(...exps);
        }
      }

      // Find workflow steps using experts that depend on this dataset
      const impactedWorkflows: any[] = [];
      const expertIds = trainedExperts.map(e => e.id);
      if (expertIds.length > 0) {
        const steps = await db.select({
          workflowId: workflowSteps.workflowId,
          expertId: workflowSteps.expertId,
        }).from(workflowSteps).where(
          sql`${workflowSteps.expertId} IN (${sql.join(expertIds.map(id => sql`${id}`), sql`, `)})`
        );
        const wfIds = [...new Set(steps.map((s: { workflowId: string }) => s.workflowId))];
        for (const wfId of wfIds) {
          const [wf] = await db.select({ id: workflows.id, name: workflows.name, status: workflows.status })
            .from(workflows).where(eq(workflows.id, wfId as string));
          if (wf) impactedWorkflows.push(wf);
        }
      }

      // Find lineage records
      const lineageRecords = await db.select().from(lineage)
        .where(or(eq(lineage.sourceId, impactId), eq(lineage.targetId, impactId)));

      return NextResponse.json({
        datasetId: impactId,
        training: trainingDeps,
        experts: trainedExperts,
        workflows: impactedWorkflows,
        lineage: lineageRecords,
        hasImpact: trainingDeps.length > 0 || trainedExperts.length > 0 || impactedWorkflows.length > 0,
      });
    }

    // Simple lineage query
    if (sourceId) {
      const rows = await db.select().from(lineage)
        .where(sourceType
          ? sql`${lineage.sourceId} = ${sourceId} AND ${lineage.sourceType} = ${sourceType}`
          : eq(lineage.sourceId, sourceId));
      return NextResponse.json({ lineage: rows });
    }

    if (targetId) {
      const rows = await db.select().from(lineage)
        .where(targetType
          ? sql`${lineage.targetId} = ${targetId} AND ${lineage.targetType} = ${targetType}`
          : eq(lineage.targetId, targetId));
      return NextResponse.json({ lineage: rows });
    }

    return NextResponse.json({ lineage: [] });
  } catch (err) {
    console.error('[lineage GET]', err);
    return NextResponse.json({ lineage: [], error: String(err) });
  }
}

/* POST /api/lineage — create a lineage record */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { sourceType, sourceId, targetType, targetId, relationship, metadata } = body;

    if (!sourceType || !sourceId || !targetType || !targetId || !relationship) {
      return NextResponse.json({ error: 'sourceType, sourceId, targetType, targetId, relationship required' }, { status: 400 });
    }

    const id = `lin-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
    const [inserted] = await db.insert(lineage).values({
      id, sourceType, sourceId, targetType, targetId, relationship,
      metadata: metadata ?? null,
    }).returning();

    return NextResponse.json({ lineage: inserted }, { status: 201 });
  } catch (err) {
    console.error('[lineage POST]', err);
    return NextResponse.json({ error: 'Failed to create lineage' }, { status: 500 });
  }
}
