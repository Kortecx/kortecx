import { NextResponse } from 'next/server';
import { db, metrics, tasks, workflowRuns, experts } from '@/lib/db';
import { sql, gte, desc } from 'drizzle-orm';

export async function GET() {
  try {
    const sevenDaysAgo = new Date(Date.now() - 7 * 86400000);

    // Aggregate weekly stats from metrics table
    const [weeklyAgg] = await db
      .select({
        totalTasks:    sql<number>`COALESCE(SUM(${metrics.tasksCompleted}), 0)`,
        totalTokens:   sql<number>`COALESCE(SUM(${metrics.tokensUsed}), 0)`,
        totalCost:     sql<number>`COALESCE(SUM(${metrics.costUsd}::numeric), 0)`,
        avgSuccess:    sql<number>`COALESCE(AVG(${metrics.successRate}::numeric), 0)`,
      })
      .from(metrics)
      .where(gte(metrics.capturedAt, sevenDaysAgo));

    // Daily breakdown from metrics
    const dailyRows = await db
      .select({
        date:   sql<string>`DATE(${metrics.capturedAt})::text`,
        tasks:  sql<number>`COALESCE(SUM(${metrics.tasksCompleted}), 0)`,
        tokens: sql<number>`COALESCE(SUM(${metrics.tokensUsed}), 0)`,
        cost:   sql<number>`COALESCE(SUM(${metrics.costUsd}::numeric), 0)`,
      })
      .from(metrics)
      .where(gte(metrics.capturedAt, sevenDaysAgo))
      .groupBy(sql`DATE(${metrics.capturedAt})`)
      .orderBy(sql`DATE(${metrics.capturedAt})`);

    // Expert performance from experts table
    const expertRows = await db
      .select({
        id:          experts.id,
        name:        experts.name,
        role:        experts.role,
        totalRuns:   experts.totalRuns,
        successRate: experts.successRate,
        avgLatencyMs: experts.avgLatencyMs,
        avgCostPerRun: experts.avgCostPerRun,
      })
      .from(experts)
      .orderBy(desc(experts.totalRuns));

    // Provider usage from experts (group by provider)
    const providerRows = await db
      .select({
        provider:   experts.providerName,
        totalRuns:  sql<number>`COALESCE(SUM(${experts.totalRuns}), 0)`,
      })
      .from(experts)
      .groupBy(experts.providerName)
      .orderBy(desc(sql`SUM(${experts.totalRuns})`));

    let totalProviderRuns = 0;
    for (const r of providerRows) totalProviderRuns += Number(r.totalRuns);
    if (totalProviderRuns === 0) totalProviderRuns = 1;

    const dailyStats = [];
    for (const r of dailyRows) {
      dailyStats.push({
        date: r.date,
        tasks: Number(r.tasks),
        tokens: Number(r.tokens),
        cost: Number(Number(r.cost).toFixed(2)),
      });
    }

    const expertPerformance = [];
    for (const e of expertRows) {
      expertPerformance.push({
        id: e.id,
        name: e.name,
        role: e.role,
        totalRuns: Number(e.totalRuns),
        successRate: Number(e.successRate),
        avgLatencyMs: Number(e.avgLatencyMs),
        avgCostPerRun: Number(e.avgCostPerRun),
      });
    }

    const providerUsage = [];
    for (const r of providerRows) {
      providerUsage.push({
        provider: r.provider,
        totalRuns: Number(r.totalRuns),
        percentage: Math.round((Number(r.totalRuns) / totalProviderRuns) * 100),
      });
    }

    return NextResponse.json({
      overview: {
        tasksThisWeek: Number(weeklyAgg.totalTasks),
        tokensThisWeek: Number(weeklyAgg.totalTokens),
        costThisWeek: Number(Number(weeklyAgg.totalCost).toFixed(2)),
        avgSuccessRate: Number(Number(weeklyAgg.avgSuccess).toFixed(4)),
      },
      dailyStats,
      expertPerformance,
      providerUsage,
    });
  } catch {
    return NextResponse.json({
      overview: { tasksThisWeek: 0, tokensThisWeek: 0, costThisWeek: 0, avgSuccessRate: 0 },
      dailyStats: [],
      expertPerformance: [],
      providerUsage: [],
    });
  }
}
