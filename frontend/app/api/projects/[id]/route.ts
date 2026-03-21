import { NextRequest, NextResponse } from 'next/server';
import { db, projects } from '@/lib/db';
import { eq } from 'drizzle-orm';
import { projectsStore } from '../store';
import { logStatus } from '@/lib/status-log';

export async function GET(_req: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;

  try {
    const [row] = await db.select().from(projects).where(eq(projects.id, id));
    if (row) return NextResponse.json(row);
  } catch {
    // DB unavailable
  }

  const project = projectsStore.find(p => p.id === id);
  if (!project) return NextResponse.json({ message: 'Not found' }, { status: 404 });
  return NextResponse.json(project);
}

export async function PUT(req: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const body = await req.json();

  try {
    const updates: Record<string, unknown> = { updatedAt: new Date() };
    if (body.name !== undefined) updates.name = body.name.trim();
    if (body.description !== undefined) updates.description = body.description?.trim() || null;
    if (body.platforms !== undefined) updates.platforms = body.platforms;
    if (body.status !== undefined) updates.status = body.status;

    const [updated] = await db.update(projects)
      .set(updates)
      .where(eq(projects.id, id))
      .returning();

    if (updated) {
      logStatus('info', `Project updated: ${id}`, 'project', { id });
      return NextResponse.json(updated);
    }
  } catch {
    // DB unavailable
  }

  // Fallback
  const idx = projectsStore.findIndex(p => p.id === id);
  if (idx === -1) return NextResponse.json({ message: 'Not found' }, { status: 404 });

  projectsStore[idx] = {
    ...projectsStore[idx],
    ...(body.name !== undefined && { name: body.name.trim() }),
    ...(body.description !== undefined && { description: body.description?.trim() || undefined }),
    ...(body.platforms !== undefined && { platforms: body.platforms }),
    ...(body.status !== undefined && { status: body.status }),
    updatedAt: new Date().toISOString(),
  };
  return NextResponse.json(projectsStore[idx]);
}

export async function DELETE(_req: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;

  try {
    const [deleted] = await db.delete(projects).where(eq(projects.id, id)).returning();
    if (deleted) {
      logStatus('info', `Project deleted: ${id}`, 'project', { id });
      return NextResponse.json({ success: true });
    }
  } catch {
    // DB unavailable
  }

  const idx = projectsStore.findIndex(p => p.id === id);
  if (idx === -1) return NextResponse.json({ message: 'Not found' }, { status: 404 });
  projectsStore.splice(idx, 1);
  return NextResponse.json({ success: true });
}
