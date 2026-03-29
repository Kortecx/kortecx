import { NextRequest, NextResponse } from 'next/server';
import { db, plans, workflows } from '@/lib/db';
import { eq, desc, and } from 'drizzle-orm';

/* GET /api/plans/versions?workflowId=xxx — list plan versions for a workflow */
export async function GET(req: NextRequest) {
  const workflowId = req.nextUrl.searchParams.get('workflowId');
  if (!workflowId) {
    return NextResponse.json({ error: 'workflowId required' }, { status: 400 });
  }

  try {
    const rows = await db.select().from(plans)
      .where(eq(plans.workflowId, workflowId))
      .orderBy(desc(plans.version));
    return NextResponse.json({ versions: rows, total: rows.length });
  } catch (err) {
    console.error('[plans/versions GET]', err);
    return NextResponse.json({ versions: [], total: 0 });
  }
}

/* PATCH /api/plans/versions — update max versions for a workflow */
export async function PATCH(req: NextRequest) {
  try {
    const body = await req.json();
    const { workflowId, maxVersions } = body;

    if (!workflowId || typeof maxVersions !== 'number') {
      return NextResponse.json({ error: 'workflowId and maxVersions required' }, { status: 400 });
    }

    const clamped = Math.max(1, Math.min(maxVersions, 50));

    await db.update(workflows).set({
      planMaxVersions: clamped,
      updatedAt: new Date(),
    }).where(eq(workflows.id, workflowId));

    // Prune excess plan versions in DB
    const allPlans = await db.select().from(plans)
      .where(and(eq(plans.workflowId, workflowId), eq(plans.planType, 'live')))
      .orderBy(desc(plans.version));

    const excess = allPlans.slice(clamped);
    for (const p of excess) {
      await db.delete(plans).where(eq(plans.id, p.id));
    }

    return NextResponse.json({ maxVersions: clamped, pruned: excess.length });
  } catch (err) {
    console.error('[plans/versions PATCH]', err);
    return NextResponse.json({ error: 'Update failed' }, { status: 500 });
  }
}
