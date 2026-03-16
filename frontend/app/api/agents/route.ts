import { NextResponse } from 'next/server';
import { db, tasks, experts } from '@/lib/db';
import { eq, inArray } from 'drizzle-orm';

export async function GET() {

  try {
    const runningTasks = await db.select().from(tasks).where(eq(tasks.status, 'running')).limit(20);
    const activeExperts = await db.select().from(experts)
      .where(inArray(experts.status, ['active', 'idle', 'training']))
      .limit(50);

    type TaskRow   = typeof runningTasks[number];
    type ExpertRow = typeof activeExperts[number];

    const taskMap = new Map<string | null, TaskRow>(
      runningTasks.map((t: TaskRow) => [t.currentExpert, t])
    );

    const agents = activeExperts.map((e: ExpertRow) => {
      const currentTask = taskMap.get(e.name);
      return {
        id:              e.id,
        name:            e.name,
        role:            e.role,
        status:          e.status,
        taskId:          currentTask?.id   ?? null,
        taskName:        currentTask?.name ?? null,
        model:           e.modelName       ?? e.modelId,
        provider:        e.providerName    ?? e.providerId,
        tokensUsed:      currentTask?.tokensUsed ?? 0,
        uptimeMin:       0,
        requestsHandled: e.totalRuns       ?? 0,
      };
    });

    const active = agents.filter((a: { status: string }) => a.status === 'active').length;
    return NextResponse.json({ agents, total: agents.length, active });
  } catch (err) {
    console.error('[agents GET]', err);
    return NextResponse.json({ agents: [], total: 0, active: 0 });
  }
}
