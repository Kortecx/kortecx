import { NextRequest, NextResponse } from 'next/server';
import { db, experts, workflows, workflowSteps, datasets, datasetSchemas, plans } from '@/lib/db';
import { eq, asc } from 'drizzle-orm';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* GET /api/export?type=expert|workflow|dataset|mcp_server&id=xxx */
export async function GET(req: NextRequest) {
  const { searchParams } = new URL(req.url);
  const type = searchParams.get('type');
  const id = searchParams.get('id');

  if (!type || !id) {
    return NextResponse.json({ error: 'Missing type or id parameter' }, { status: 400 });
  }

  try {
    switch (type) {
      case 'expert': {
        const [expert] = await db.select().from(experts).where(eq(experts.id, id));
        if (!expert) return NextResponse.json({ error: 'Expert not found' }, { status: 404 });
        return NextResponse.json({
          expert,
        });
      }

      case 'workflow': {
        const [workflow] = await db.select().from(workflows).where(eq(workflows.id, id));
        if (!workflow) return NextResponse.json({ error: 'Workflow not found' }, { status: 404 });

        const steps = await db.select().from(workflowSteps)
          .where(eq(workflowSteps.workflowId, id))
          .orderBy(asc(workflowSteps.order));

        let plan = null;
        if (workflow.activePlanId) {
          const [p] = await db.select().from(plans).where(eq(plans.id, workflow.activePlanId));
          plan = p || null;
        }

        return NextResponse.json({
          workflow,
          steps,
          plan,
        });
      }

      case 'dataset': {
        const [dataset] = await db.select().from(datasets).where(eq(datasets.id, id));
        if (!dataset) return NextResponse.json({ error: 'Dataset not found' }, { status: 404 });

        const schema = await db.select().from(datasetSchemas)
          .where(eq(datasetSchemas.datasetId, id));

        return NextResponse.json({
          dataset,
          schema,
        });
      }

      case 'mcp_server': {
        // MCP servers are managed by the engine
        const res = await fetch(`${ENGINE_URL}/api/mcp/server/${encodeURIComponent(id)}`);
        if (!res.ok) {
          return NextResponse.json({ error: 'MCP server not found' }, { status: 404 });
        }
        const server = await res.json();
        return NextResponse.json({ server });
      }

      default:
        return NextResponse.json({ error: `Unsupported export type: ${type}` }, { status: 400 });
    }
  } catch (err) {
    console.error('Export error:', err);
    return NextResponse.json({ error: 'Export failed' }, { status: 500 });
  }
}
