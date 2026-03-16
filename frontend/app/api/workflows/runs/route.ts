import { NextRequest, NextResponse } from 'next/server';
import { db, workflowRuns } from '@/lib/db';
import { desc, eq } from 'drizzle-orm';

export async function GET(req: NextRequest) {

  const { searchParams } = new URL(req.url);
  const workflowId = searchParams.get('workflowId');
  const limit = Number(searchParams.get('limit') ?? 50);

  try {
    const query = db.select().from(workflowRuns);
    if (workflowId) query.where(eq(workflowRuns.workflowId, workflowId));
    const rows = await query.orderBy(desc(workflowRuns.createdAt)).limit(limit);

    return NextResponse.json({ runs: rows, total: rows.length });
  } catch (err) {
    console.error('[workflow runs GET]', err);
    return NextResponse.json({ runs: [], total: 0 });
  }
}
