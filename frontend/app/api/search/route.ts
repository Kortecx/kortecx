import { NextRequest, NextResponse } from 'next/server';
import {
  db, experts, workflows, tasks, trainingJobs, datasets, alerts, projects,
  type Expert, type Workflow, type Task, type TrainingJob, type Dataset, type Alert, type Project,
} from '@/lib/db';
import { ilike, or, desc, sql } from 'drizzle-orm';

/* ─── Search result shape ────────────────────────────── */
interface SearchResult {
  id: string;
  type: 'expert' | 'workflow' | 'task' | 'training' | 'dataset' | 'alert' | 'project';
  name: string;
  description: string | null;
  status: string | null;
  meta: Record<string, unknown>;
  href: string;
  updatedAt: string | null;
}

/* GET /api/search?q=<query>&type=<optional filter>&limit=<optional> */
export async function GET(req: NextRequest) {
  const { searchParams } = req.nextUrl;
  const q     = searchParams.get('q')?.trim();
  const type  = searchParams.get('type');       // filter to one category
  const limit = Math.min(Number(searchParams.get('limit') ?? 25), 100);

  if (!q || q.length < 1) {
    return NextResponse.json({ results: [], total: 0 });
  }

  const pattern = `%${q}%`;

  try {
    const results: SearchResult[] = [];

    // Run all searches in parallel
    const searches: Promise<void>[] = [];

    /* ── Experts ─────────────────────────────────── */
    if (!type || type === 'expert') {
      searches.push(
        db.select()
          .from(experts)
          .where(or(
            ilike(experts.name, pattern),
            ilike(experts.description, pattern),
            sql`array_to_string(${experts.tags}, ',') ILIKE ${pattern}`,
          )!)
          .orderBy(desc(experts.updatedAt))
          .limit(limit)
          .then((rows: Expert[]) => {
            for (const r of rows) {
              results.push({
                id: r.id,
                type: 'expert',
                name: r.name,
                description: r.description,
                status: r.status,
                meta: { role: r.role, rating: r.rating, tags: r.tags },
                href: `/experts?tab=mine&highlight=${r.id}`,
                updatedAt: r.updatedAt?.toISOString() ?? null,
              });
            }
          }),
      );
    }

    /* ── Workflows ────────────────────────────────── */
    if (!type || type === 'workflow') {
      searches.push(
        db.select()
          .from(workflows)
          .where(or(
            ilike(workflows.name, pattern),
            ilike(workflows.description, pattern),
            ilike(workflows.goalStatement, pattern),
            sql`array_to_string(${workflows.tags}, ',') ILIKE ${pattern}`,
          )!)
          .orderBy(desc(workflows.updatedAt))
          .limit(limit)
          .then((rows: Workflow[]) => {
            for (const r of rows) {
              results.push({
                id: r.id,
                type: 'workflow',
                name: r.name,
                description: r.description,
                status: r.status,
                meta: { totalRuns: r.totalRuns, tags: r.tags, isTemplate: r.isTemplate },
                href: `/workflow?id=${r.id}`,
                updatedAt: r.updatedAt?.toISOString() ?? null,
              });
            }
          }),
      );
    }

    /* ── Tasks ────────────────────────────────────── */
    if (!type || type === 'task') {
      searches.push(
        db.select()
          .from(tasks)
          .where(or(
            ilike(tasks.name, pattern),
            ilike(tasks.workflowName, pattern),
            ilike(tasks.currentExpert, pattern),
            ilike(tasks.input, pattern),
          )!)
          .orderBy(desc(tasks.updatedAt))
          .limit(limit)
          .then((rows: Task[]) => {
            for (const r of rows) {
              results.push({
                id: r.id,
                type: 'task',
                name: r.name,
                description: r.workflowName ? `Workflow: ${r.workflowName}` : null,
                status: r.status,
                meta: { priority: r.priority, progress: r.progress, currentExpert: r.currentExpert },
                href: '/dashboard',
                updatedAt: r.updatedAt?.toISOString() ?? null,
              });
            }
          }),
      );
    }

    /* ── Training Jobs ────────────────────────────── */
    if (!type || type === 'training') {
      searches.push(
        db.select()
          .from(trainingJobs)
          .where(or(
            ilike(trainingJobs.name, pattern),
            ilike(trainingJobs.baseModelId, pattern),
          )!)
          .orderBy(desc(trainingJobs.createdAt))
          .limit(limit)
          .then((rows: TrainingJob[]) => {
            for (const r of rows) {
              results.push({
                id: r.id,
                type: 'training',
                name: r.name,
                description: `Base model: ${r.baseModelId}`,
                status: r.status,
                meta: { progress: r.progress, epochs: r.epochs, currentEpoch: r.currentEpoch },
                href: '/training',
                updatedAt: r.createdAt?.toISOString() ?? null,
              });
            }
          }),
      );
    }

    /* ── Datasets ─────────────────────────────────── */
    if (!type || type === 'dataset') {
      searches.push(
        db.select()
          .from(datasets)
          .where(or(
            ilike(datasets.name, pattern),
            ilike(datasets.description, pattern),
            sql`array_to_string(${datasets.tags}, ',') ILIKE ${pattern}`,
            sql`array_to_string(${datasets.categories}, ',') ILIKE ${pattern}`,
          )!)
          .orderBy(desc(datasets.updatedAt))
          .limit(limit)
          .then((rows: Dataset[]) => {
            for (const r of rows) {
              results.push({
                id: r.id,
                type: 'dataset',
                name: r.name,
                description: r.description,
                status: r.status,
                meta: { sampleCount: r.sampleCount, qualityScore: r.qualityScore, tags: r.tags },
                href: '/training/data',
                updatedAt: r.updatedAt?.toISOString() ?? null,
              });
            }
          }),
      );
    }

    /* ── Alerts ───────────────────────────────────── */
    if (!type || type === 'alert') {
      searches.push(
        db.select()
          .from(alerts)
          .where(or(
            ilike(alerts.title, pattern),
            ilike(alerts.message, pattern),
          )!)
          .orderBy(desc(alerts.createdAt))
          .limit(limit)
          .then((rows: Alert[]) => {
            for (const r of rows) {
              results.push({
                id: r.id,
                type: 'alert',
                name: r.title,
                description: r.message,
                status: r.severity,
                meta: { acknowledged: r.acknowledged, severity: r.severity },
                href: '/monitoring/alerts',
                updatedAt: r.createdAt?.toISOString() ?? null,
              });
            }
          }),
      );
    }

    /* ── Projects ─────────────────────────────────── */
    if (!type || type === 'project') {
      searches.push(
        db.select()
          .from(projects)
          .where(or(
            ilike(projects.name, pattern),
            ilike(projects.description, pattern),
          )!)
          .orderBy(desc(projects.updatedAt))
          .limit(limit)
          .then((rows: Project[]) => {
            for (const r of rows) {
              results.push({
                id: r.id,
                type: 'project',
                name: r.name,
                description: r.description,
                status: r.status,
                meta: { platforms: r.platforms, postsCount: r.postsCount },
                href: '/projects',
                updatedAt: r.updatedAt?.toISOString() ?? null,
              });
            }
          }),
      );
    }

    await Promise.all(searches);

    // Sort all results by updatedAt descending (most recent first)
    results.sort((a, b) => {
      if (!a.updatedAt) return 1;
      if (!b.updatedAt) return -1;
      return new Date(b.updatedAt).getTime() - new Date(a.updatedAt).getTime();
    });

    // Apply global limit
    const trimmed = results.slice(0, limit);

    return NextResponse.json({ results: trimmed, total: results.length });
  } catch (err) {
    console.error('[search GET]', err);
    return NextResponse.json({ results: [], total: 0 });
  }
}
