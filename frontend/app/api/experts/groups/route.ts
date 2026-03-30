import { NextRequest, NextResponse } from 'next/server';
import { db, agentGroups } from '@/lib/db';
import { eq, desc } from 'drizzle-orm';
import crypto from 'crypto';

/* GET /api/experts/groups — list all agent groups */
export async function GET() {
  try {
    const rows = await db.select().from(agentGroups).orderBy(desc(agentGroups.createdAt));
    return NextResponse.json({ groups: rows, total: rows.length });
  } catch (err) {
    console.error('[experts/groups GET]', err);
    return NextResponse.json({ groups: [], total: 0 });
  }
}

/* POST /api/experts/groups — create agent group */
export async function POST(req: NextRequest) {
  try {
    const { name, description, agentIds } = await req.json();
    if (!name?.trim()) return NextResponse.json({ error: 'name required' }, { status: 400 });
    const id = `grp-${crypto.randomUUID().slice(0, 8)}`;
    const [row] = await db.insert(agentGroups).values({
      id, name: name.trim(), description: description?.trim() || null, agentIds: agentIds ?? [],
    }).returning();
    return NextResponse.json({ group: row }, { status: 201 });
  } catch (err) {
    console.error('[experts/groups POST]', err);
    return NextResponse.json({ error: 'Failed to create group' }, { status: 500 });
  }
}

/* DELETE /api/experts/groups?id={id} — delete agent group */
export async function DELETE(req: NextRequest) {
  const id = req.nextUrl.searchParams.get('id');
  if (!id) return NextResponse.json({ error: 'id required' }, { status: 400 });
  try {
    await db.delete(agentGroups).where(eq(agentGroups.id, id));
    return NextResponse.json({ deleted: true });
  } catch (err) {
    console.error('[experts/groups DELETE]', err);
    return NextResponse.json({ error: 'Failed to delete' }, { status: 500 });
  }
}
