import { NextRequest, NextResponse } from 'next/server';
import { db, projects } from '@/lib/db';
import { desc, eq, ilike, or, gte, sql } from 'drizzle-orm';
import { projectsStore } from './store';
import type { ProjectRecord } from '@/lib/api-client';

export async function GET(req: NextRequest) {
  const { searchParams } = new URL(req.url);
  const status = searchParams.get('status');
  const q = searchParams.get('q');
  const range = searchParams.get('range');

  try {
    const conditions = [];
    if (status && status !== 'all') conditions.push(eq(projects.status, status));
    if (q) {
      conditions.push(
        or(
          ilike(projects.name, `%${q}%`),
          ilike(projects.description, `%${q}%`),
        )!
      );
    }
    if (range && range !== 'all') {
      const days = parseInt(range, 10);
      if (!isNaN(days)) {
        conditions.push(gte(projects.updatedAt, new Date(Date.now() - days * 86400000)));
      }
    }

    const rows = conditions.length > 0
      ? await db.select().from(projects)
          .where(sql`${sql.join(conditions, sql` AND `)}`)
          .orderBy(desc(projects.updatedAt))
      : await db.select().from(projects).orderBy(desc(projects.updatedAt));

    if (rows.length > 0) {
      const mapped = [];
      for (const r of rows) {
        mapped.push({
          id: r.id,
          name: r.name,
          description: r.description,
          createdAt: r.createdAt.toISOString(),
          updatedAt: r.updatedAt.toISOString(),
          platforms: r.platforms ?? [],
          postsCount: r.postsCount ?? 0,
          status: r.status ?? 'active',
        });
      }
      return NextResponse.json(mapped);
    }
  } catch {
    // DB unavailable
  }

  // Fallback to in-memory store
  let result = [...projectsStore];
  if (status && status !== 'all') result = result.filter(p => p.status === status);
  if (q) {
    const query = q.toLowerCase();
    result = result.filter(p =>
      p.name.toLowerCase().includes(query) ||
      (p.description ?? '').toLowerCase().includes(query)
    );
  }
  return NextResponse.json(result.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt)));
}

export async function POST(req: NextRequest) {
  const body = await req.json();

  if (!body.name?.trim()) {
    return NextResponse.json({ message: 'Project name is required' }, { status: 400 });
  }

  const id = `proj-${Date.now()}`;
  const now = new Date();

  try {
    const [inserted] = await db.insert(projects).values({
      id,
      name: body.name.trim(),
      description: body.description?.trim() || null,
      status: body.status ?? 'active',
      platforms: Array.isArray(body.platforms) ? body.platforms : [],
      postsCount: 0,
    }).returning();

    return NextResponse.json({
      id: inserted.id,
      name: inserted.name,
      description: inserted.description,
      createdAt: inserted.createdAt.toISOString(),
      updatedAt: inserted.updatedAt.toISOString(),
      platforms: inserted.platforms ?? [],
      postsCount: inserted.postsCount ?? 0,
      status: inserted.status,
    }, { status: 201 });
  } catch {
    // Fallback to in-memory
    const project: ProjectRecord = {
      id,
      name: body.name.trim(),
      description: body.description?.trim() || undefined,
      createdAt: now.toISOString(),
      updatedAt: now.toISOString(),
      platforms: Array.isArray(body.platforms) ? body.platforms : [],
      postsCount: 0,
      status: body.status ?? 'draft',
    };
    projectsStore.unshift(project);
    return NextResponse.json(project, { status: 201 });
  }
}
