import { NextRequest, NextResponse } from 'next/server';
import { db, alerts } from '@/lib/db';
import { eq, desc } from 'drizzle-orm';

/* GET /api/alerts — optional ?severity=warning */
export async function GET(req: NextRequest) {
  const severity = req.nextUrl.searchParams.get('severity');

  try {
    const rows = severity
      ? await db.select().from(alerts).where(eq(alerts.severity, severity)).orderBy(desc(alerts.createdAt))
      : await db.select().from(alerts).orderBy(desc(alerts.createdAt));

    return NextResponse.json({ alerts: rows });
  } catch {
    return NextResponse.json({ alerts: [] });
  }
}

/* POST /api/alerts — create or acknowledge/resolve */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();

    // Acknowledge or resolve existing alert
    if (body.alertId && body.action) {
      const updates: Record<string, unknown> = {};
      if (body.action === 'acknowledge') {
        updates.acknowledged = true;
        updates.acknowledgedAt = new Date();
      }
      if (body.action === 'resolve') {
        updates.resolvedAt = new Date();
      }

      const [updated] = await db.update(alerts)
        .set(updates)
        .where(eq(alerts.id, body.alertId))
        .returning();

      return NextResponse.json({ success: true, alert: updated });
    }

    // Create new alert
    const { severity, title, message, providerId, expertId } = body;
    if (!severity || !title || !message) {
      return NextResponse.json({ error: 'severity, title, message are required' }, { status: 400 });
    }

    const id = `alert-${Date.now()}`;
    const [inserted] = await db.insert(alerts).values({
      id, severity, title, message,
      providerId: providerId ?? null,
      expertId: expertId ?? null,
    }).returning();

    return NextResponse.json({ alert: inserted }, { status: 201 });
  } catch (err) {
    const message = err instanceof Error ? err.message : 'Invalid request body';
    return NextResponse.json({ error: message }, { status: 400 });
  }
}
