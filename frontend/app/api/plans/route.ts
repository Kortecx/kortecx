import { NextRequest, NextResponse } from 'next/server';
import { db, plans } from '@/lib/db';
import { eq, desc } from 'drizzle-orm';

/* GET /api/plans — list plans, optionally filter by workflowId */
export async function GET(req: NextRequest) {
  const { searchParams } = req.nextUrl;
  const workflowId = searchParams.get('workflowId');

  try {
    const rows = workflowId
      ? await db.select().from(plans).where(eq(plans.workflowId, workflowId)).orderBy(desc(plans.createdAt))
      : await db.select().from(plans).orderBy(desc(plans.createdAt));
    return NextResponse.json({ plans: rows, total: rows.length });
  } catch (err) {
    console.error('[plans GET]', err);
    return NextResponse.json({ plans: [], total: 0 });
  }
}

/* POST /api/plans — create a new plan */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { workflowId, name, description, dag, generatedBy, modelUsed } = body;

    if (!name?.trim()) {
      return NextResponse.json({ error: 'Plan name is required' }, { status: 400 });
    }

    const id = `plan-${Date.now()}`;
    const [inserted] = await db.insert(plans).values({
      id,
      workflowId: workflowId ?? null,
      name: name.trim(),
      description: description?.trim() ?? null,
      dag: dag ?? { nodes: [], edges: [] },
      status: 'draft',
      generatedBy: generatedBy ?? 'user',
      modelUsed: modelUsed ?? null,
    }).returning();

    return NextResponse.json({ plan: inserted }, { status: 201 });
  } catch (err) {
    console.error('[plans POST]', err);
    return NextResponse.json({ error: 'Failed to create plan' }, { status: 500 });
  }
}

/* PATCH /api/plans — update plan (DAG, status, positions) */
export async function PATCH(req: NextRequest) {
  try {
    const body = await req.json();
    const { id, ...updates } = body;

    if (!id) return NextResponse.json({ error: 'Plan id required' }, { status: 400 });

    const values: Record<string, unknown> = { updatedAt: new Date() };
    if (updates.name !== undefined) values.name = updates.name;
    if (updates.description !== undefined) values.description = updates.description;
    if (updates.dag !== undefined) values.dag = updates.dag;
    if (updates.status !== undefined) values.status = updates.status;

    const [updated] = await db.update(plans).set(values).where(eq(plans.id, id)).returning();
    return NextResponse.json({ plan: updated });
  } catch (err) {
    console.error('[plans PATCH]', err);
    return NextResponse.json({ error: 'Update failed' }, { status: 500 });
  }
}

/* DELETE /api/plans */
export async function DELETE(req: NextRequest) {
  const id = req.nextUrl.searchParams.get('id');
  if (!id) return NextResponse.json({ error: 'Plan id required' }, { status: 400 });

  try {
    await db.delete(plans).where(eq(plans.id, id));
    return NextResponse.json({ deleted: true });
  } catch (err) {
    console.error('[plans DELETE]', err);
    return NextResponse.json({ error: 'Delete failed' }, { status: 500 });
  }
}
