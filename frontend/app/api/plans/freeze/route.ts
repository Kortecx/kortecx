import { NextRequest, NextResponse } from 'next/server';
import { db, plans, workflows } from '@/lib/db';
import { eq } from 'drizzle-orm';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* POST /api/plans/freeze — freeze, unfreeze, or refreeze a workflow plan */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { workflowId, action, planId } = body;

    if (!workflowId) {
      return NextResponse.json({ error: 'workflowId required' }, { status: 400 });
    }

    // Get workflow
    const [wf] = await db.select().from(workflows).where(eq(workflows.id, workflowId));
    if (!wf) {
      return NextResponse.json({ error: 'Workflow not found' }, { status: 404 });
    }

    const slug = (wf.name || 'untitled').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');

    if (action === 'freeze') {
      // Call engine to copy LIVE → FREEZE
      const engineRes = await fetch(`${ENGINE_URL}/api/plans/freeze`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ workflowSlug: slug, action: 'freeze' }),
      });
      const engineData = await engineRes.json();

      if (!engineData.frozen) {
        return NextResponse.json({ error: engineData.error || 'No LIVE plan to freeze' }, { status: 400 });
      }

      // Update workflow DB
      const activePlan = wf.activePlanId as string | null;
      await db.update(workflows).set({
        planFrozen: true,
        frozenPlanId: activePlan,
        updatedAt: new Date(),
      }).where(eq(workflows.id, workflowId));

      // Update plan record if exists
      if (activePlan) {
        await db.update(plans).set({
          planType: 'frozen',
          frozenAt: new Date(),
          updatedAt: new Date(),
        }).where(eq(plans.id, activePlan));
      }

      return NextResponse.json({ frozen: true, workflowId });

    } else if (action === 'unfreeze') {
      await fetch(`${ENGINE_URL}/api/plans/freeze`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ workflowSlug: slug, action: 'unfreeze' }),
      });

      const frozenId = wf.frozenPlanId as string | null;
      await db.update(workflows).set({
        planFrozen: false,
        frozenPlanId: null,
        updatedAt: new Date(),
      }).where(eq(workflows.id, workflowId));

      // Reset plan type back to live
      if (frozenId) {
        await db.update(plans).set({
          planType: 'live',
          frozenAt: null,
          updatedAt: new Date(),
        }).where(eq(plans.id, frozenId));
      }

      return NextResponse.json({ frozen: false, workflowId });

    } else if (action === 'refreeze') {
      await fetch(`${ENGINE_URL}/api/plans/freeze`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ workflowSlug: slug, action: 'refreeze', version: null }),
      });

      const newPlanId = planId || wf.activePlanId;
      await db.update(workflows).set({
        planFrozen: true,
        frozenPlanId: newPlanId as string,
        updatedAt: new Date(),
      }).where(eq(workflows.id, workflowId));

      if (newPlanId) {
        await db.update(plans).set({
          planType: 'frozen',
          frozenAt: new Date(),
          updatedAt: new Date(),
        }).where(eq(plans.id, newPlanId as string));
      }

      return NextResponse.json({ frozen: true, workflowId, refrozen: true });

    } else {
      return NextResponse.json({ error: `Unknown action: ${action}` }, { status: 400 });
    }
  } catch (err) {
    console.error('[plans/freeze POST]', err);
    return NextResponse.json({ error: 'Freeze operation failed' }, { status: 500 });
  }
}
