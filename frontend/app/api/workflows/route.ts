import { NextRequest, NextResponse } from 'next/server';
import { db, workflows, workflowSteps, workflowRuns } from '@/lib/db';
import { desc, eq, asc } from 'drizzle-orm';
import { nanoid } from '../tasks/nanoid';

function buildStepValues(workflowId: string, steps: Record<string, unknown>[]) {
  return steps.map((s, i) => ({
    id:                 `ws-${nanoid()}`,
    workflowId,
    order:              i + 1,
    name:               (s.name as string) || null,
    description:        (s.description as string) || null,
    expertId:           (s.expertId as string) || null,
    taskDescription:    (s.taskDescription as string) || '',
    systemInstructions: (s.systemInstructions as string) || null,
    voiceCommand:       (s.voiceCommand as string) || null,
    fileLocations:      (s.fileLocations as string[]) || [],
    stepFileUrls:       (s.stepFileNames as string[]) || (s.stepFileUrls as string[]) || [],
    stepImageUrls:      (s.stepImageNames as string[]) || (s.stepImageUrls as string[]) || [],
    integrations:       s.integrations || null,
    modelSource:        (s.modelSource as string) || 'provider',
    localModelConfig:   s.localModel || s.localModelConfig || null,
    connectionType:     (s.connectionType as string) || 'sequential',
    shareMemory:        s.shareMemory !== false,
    temperature:        s.temperature != null ? String(s.temperature) : '0.7',
    maxTokens:          (s.maxTokens as number) || 4096,
  }));
}

/* GET /api/workflows — query params: templates, id */
export async function GET(req: NextRequest) {
  const { searchParams } = new URL(req.url);
  const templatesOnly = searchParams.get('templates') === '1';
  const id = searchParams.get('id');

  try {
    // Single workflow by ID — include steps
    if (id) {
      const [row] = await db.select().from(workflows).where(eq(workflows.id, id));
      if (!row) return NextResponse.json({ error: 'Workflow not found' }, { status: 404 });

      const steps = await db.select().from(workflowSteps)
        .where(eq(workflowSteps.workflowId, id))
        .orderBy(asc(workflowSteps.order));

      return NextResponse.json({ workflow: row, steps });
    }

    const rows = await db
      .select()
      .from(workflows)
      .where(templatesOnly ? eq(workflows.isTemplate, true) : undefined)
      .orderBy(desc(workflows.updatedAt))
      .limit(50);

    return NextResponse.json({ workflows: rows, total: rows.length });
  } catch (err) {
    console.error('[workflows GET]', err);
    return NextResponse.json({ error: 'Database error' }, { status: 500 });
  }
}

/* POST /api/workflows — create workflow with steps */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const {
      name, description, goalStatement, goalFileUrl, inputFileUrls,
      tags, isTemplate, templateCategory, steps,
    } = body;

    if (!name?.trim()) {
      return NextResponse.json({ error: 'Workflow name is required' }, { status: 400 });
    }

    const workflowId = `wf-${nanoid()}`;

    const [row] = await db.insert(workflows).values({
      id:               workflowId,
      name:             name.trim(),
      description:      description?.trim() ?? null,
      goalStatement:    goalStatement ?? null,
      goalFileUrl:      goalFileUrl ?? null,
      inputFileUrls:    inputFileUrls ?? [],
      status:           'draft',
      tags:             tags ?? [],
      isTemplate:       isTemplate ?? false,
      templateCategory: templateCategory ?? null,
    }).returning();

    // Persist steps if provided
    if (steps && Array.isArray(steps) && steps.length > 0) {
      await db.insert(workflowSteps).values(buildStepValues(workflowId, steps));
    }

    return NextResponse.json({ workflow: row }, { status: 201 });
  } catch (err) {
    console.error('[workflows POST]', err);
    return NextResponse.json({ error: 'Database error' }, { status: 500 });
  }
}

/* PATCH /api/workflows — update workflow and optionally replace steps */
export async function PATCH(req: NextRequest) {
  try {
    const body = await req.json();
    const { id, steps: newSteps, ...updates } = body;

    if (!id) {
      return NextResponse.json({ error: 'Workflow id is required' }, { status: 400 });
    }

    const [existing] = await db.select().from(workflows).where(eq(workflows.id, id));
    if (!existing) {
      return NextResponse.json({ error: 'Workflow not found' }, { status: 404 });
    }

    // Build update values
    const values: Record<string, unknown> = { updatedAt: new Date() };
    if (updates.name !== undefined)              values.name = updates.name.trim();
    if (updates.description !== undefined)       values.description = updates.description?.trim() || null;
    if (updates.goalStatement !== undefined)      values.goalStatement = updates.goalStatement;
    if (updates.goalFileUrl !== undefined)        values.goalFileUrl = updates.goalFileUrl;
    if (updates.inputFileUrls !== undefined)      values.inputFileUrls = updates.inputFileUrls;
    if (updates.status !== undefined)            values.status = updates.status;
    if (updates.tags !== undefined)              values.tags = updates.tags;
    if (updates.isTemplate !== undefined)        values.isTemplate = updates.isTemplate;
    if (updates.templateCategory !== undefined)  values.templateCategory = updates.templateCategory;
    if (updates.estimatedTokens !== undefined)   values.estimatedTokens = updates.estimatedTokens;
    if (updates.estimatedCostUsd !== undefined)   values.estimatedCostUsd = updates.estimatedCostUsd;
    if (updates.estimatedDurationSec !== undefined) values.estimatedDurationSec = updates.estimatedDurationSec;
    if (updates.metadata !== undefined)            values.metadata = updates.metadata;

    const [updated] = await db.update(workflows)
      .set(values)
      .where(eq(workflows.id, id))
      .returning();

    // Replace steps if provided
    if (newSteps && Array.isArray(newSteps)) {
      await db.delete(workflowSteps).where(eq(workflowSteps.workflowId, id));
      if (newSteps.length > 0) {
        await db.insert(workflowSteps).values(buildStepValues(id, newSteps));
      }
    }

    return NextResponse.json({ workflow: updated });
  } catch (err) {
    console.error('[workflows PATCH]', err);
    return NextResponse.json({ error: 'Update failed' }, { status: 500 });
  }
}

/* DELETE /api/workflows — remove workflow and its steps */
export async function DELETE(req: NextRequest) {
  try {
    const { searchParams } = new URL(req.url);
    const id = searchParams.get('id');

    if (!id) {
      return NextResponse.json({ error: 'Workflow id is required' }, { status: 400 });
    }

    // Delete steps first (cascade)
    await db.delete(workflowSteps).where(eq(workflowSteps.workflowId, id));

    const [deleted] = await db.delete(workflows).where(eq(workflows.id, id)).returning();
    if (!deleted) {
      return NextResponse.json({ error: 'Workflow not found' }, { status: 404 });
    }

    return NextResponse.json({ deleted: true, id });
  } catch (err) {
    console.error('[workflows DELETE]', err);
    return NextResponse.json({ error: 'Delete failed' }, { status: 500 });
  }
}
