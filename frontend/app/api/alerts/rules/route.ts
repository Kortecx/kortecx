import { NextRequest, NextResponse } from 'next/server';
import { db, alertRules, lineage } from '@/lib/db';
import { eq, desc } from 'drizzle-orm';
import { nanoid } from '../../tasks/nanoid';

/* GET /api/alerts/rules — list all alert rules */
export async function GET() {
  try {
    const rules = await db.select().from(alertRules).orderBy(desc(alertRules.createdAt));
    return NextResponse.json({ rules });
  } catch (err) {
    console.error('Failed to list alert rules:', err);
    return NextResponse.json({ error: 'Failed to list alert rules' }, { status: 500 });
  }
}

/* POST /api/alerts/rules — create a new alert rule */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { name, description, triggerType, conditions, notificationConfig, severity, enabled, cooldownMinutes } = body;

    if (!name || !triggerType || !conditions || !notificationConfig) {
      return NextResponse.json({ error: 'Missing required fields: name, triggerType, conditions, notificationConfig' }, { status: 400 });
    }

    const id = `ar-${nanoid()}`;
    const [rule] = await db.insert(alertRules).values({
      id,
      name,
      description: description || null,
      triggerType,
      conditions,
      notificationConfig,
      severity: severity || 'warning',
      enabled: enabled !== false,
      cooldownMinutes: cooldownMinutes || 15,
    }).returning();

    // Create lineage records for referenced entities
    if (conditions.workflowId) {
      await db.insert(lineage).values({
        id: `lin-${nanoid()}`,
        sourceType: 'alert_rule',
        sourceId: id,
        targetType: 'workflow',
        targetId: conditions.workflowId,
        relationship: 'depends_on',
        metadata: { triggerType },
      });
    }
    if (conditions.expertId) {
      await db.insert(lineage).values({
        id: `lin-${nanoid()}`,
        sourceType: 'alert_rule',
        sourceId: id,
        targetType: 'expert',
        targetId: conditions.expertId,
        relationship: 'depends_on',
        metadata: { triggerType },
      });
    }

    return NextResponse.json({ rule });
  } catch (err) {
    console.error('Failed to create alert rule:', err);
    return NextResponse.json({ error: 'Failed to create alert rule' }, { status: 500 });
  }
}

/* PATCH /api/alerts/rules — update an existing alert rule */
export async function PATCH(req: NextRequest) {
  try {
    const body = await req.json();
    const { id, ...updates } = body;

    if (!id) {
      return NextResponse.json({ error: 'Missing rule id' }, { status: 400 });
    }

    const [existing] = await db.select().from(alertRules).where(eq(alertRules.id, id));
    if (!existing) {
      return NextResponse.json({ error: 'Alert rule not found' }, { status: 404 });
    }

    const updateValues: Record<string, unknown> = { updatedAt: new Date() };
    if (updates.name !== undefined) updateValues.name = updates.name;
    if (updates.description !== undefined) updateValues.description = updates.description;
    if (updates.triggerType !== undefined) updateValues.triggerType = updates.triggerType;
    if (updates.conditions !== undefined) updateValues.conditions = updates.conditions;
    if (updates.notificationConfig !== undefined) updateValues.notificationConfig = updates.notificationConfig;
    if (updates.severity !== undefined) updateValues.severity = updates.severity;
    if (updates.enabled !== undefined) updateValues.enabled = updates.enabled;
    if (updates.cooldownMinutes !== undefined) updateValues.cooldownMinutes = updates.cooldownMinutes;

    const [rule] = await db.update(alertRules).set(updateValues).where(eq(alertRules.id, id)).returning();
    return NextResponse.json({ rule });
  } catch (err) {
    console.error('Failed to update alert rule:', err);
    return NextResponse.json({ error: 'Failed to update alert rule' }, { status: 500 });
  }
}

/* DELETE /api/alerts/rules?id=xxx — delete an alert rule */
export async function DELETE(req: NextRequest) {
  const { searchParams } = new URL(req.url);
  const id = searchParams.get('id');

  if (!id) {
    return NextResponse.json({ error: 'Missing rule id' }, { status: 400 });
  }

  try {
    await db.delete(alertRules).where(eq(alertRules.id, id));
    return NextResponse.json({ deleted: true });
  } catch (err) {
    console.error('Failed to delete alert rule:', err);
    return NextResponse.json({ error: 'Failed to delete alert rule' }, { status: 500 });
  }
}
