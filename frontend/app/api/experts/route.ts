import { NextRequest, NextResponse } from 'next/server';
import { db, experts } from '@/lib/db';
import { eq, ilike, or, desc, asc, sql } from 'drizzle-orm';

/* GET /api/experts — query params: role, status, search, sort, id */
export async function GET(req: NextRequest) {
  const { searchParams } = req.nextUrl;
  const id     = searchParams.get('id');
  const role   = searchParams.get('role');
  const status = searchParams.get('status');
  const search = searchParams.get('search');
  const sort   = searchParams.get('sort') ?? 'rating';

  try {
    // Single expert by ID
    if (id) {
      const [row] = await db.select().from(experts).where(eq(experts.id, id));
      if (!row) return NextResponse.json({ error: 'Expert not found' }, { status: 404 });
      return NextResponse.json({ expert: row });
    }

    const conditions = [];
    if (role)   conditions.push(eq(experts.role, role));
    if (status) conditions.push(eq(experts.status, status));
    if (search) {
      conditions.push(
        or(
          ilike(experts.name, `%${search}%`),
          ilike(experts.description, `%${search}%`),
        )!,
      );
    }

    const orderBy = sort === 'runs'
      ? desc(experts.totalRuns)
      : sort === 'name'
      ? asc(experts.name)
      : sort === 'cost'
      ? asc(experts.avgCostPerRun)
      : desc(experts.rating);

    const rows = conditions.length > 0
      ? await db.select().from(experts)
          .where(sql`${sql.join(conditions, sql` AND `)}`)
          .orderBy(orderBy)
      : await db.select().from(experts).orderBy(orderBy);

    return NextResponse.json({ experts: rows, total: rows.length });
  } catch (err) {
    console.error('[experts GET]', err);
    return NextResponse.json({ experts: [], total: 0 });
  }
}

/* POST /api/experts — Deploy/create a new expert */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const {
      name, role, modelId, providerId, systemPrompt, temperature,
      maxTokens, description, tags, isPublic, modelSource, localModelConfig,
    } = body;

    if (!name?.trim()) {
      return NextResponse.json({ error: 'Expert name is required' }, { status: 400 });
    }
    if (!role) {
      return NextResponse.json({ error: 'Expert role is required' }, { status: 400 });
    }

    const isLocal = modelSource === 'local';

    // Local experts use the model name from localModelConfig
    const resolvedModelId = isLocal
      ? (localModelConfig?.model || localModelConfig?.modelName || 'llama3.1:8b')
      : modelId;
    const resolvedProviderId = isLocal
      ? (localModelConfig?.engine || 'ollama')
      : providerId;

    if (!isLocal && (!modelId || !providerId)) {
      return NextResponse.json({ error: 'modelId and providerId required for provider experts' }, { status: 400 });
    }

    const id = `exp-${Date.now()}`;
    const [inserted] = await db.insert(experts).values({
      id,
      name: name.trim(),
      role,
      modelId:       resolvedModelId,
      providerId:    resolvedProviderId,
      modelName:     isLocal ? resolvedModelId : (modelId || ''),
      providerName:  isLocal ? (localModelConfig?.engine || 'ollama') : (providerId || ''),
      modelSource:   modelSource || 'provider',
      localModelConfig: isLocal ? localModelConfig : null,
      description:   description?.trim() || null,
      systemPrompt:  systemPrompt?.trim() || null,
      temperature:   String(temperature ?? 0.7),
      maxTokens:     maxTokens ?? 4096,
      status:        'deploying',
      version:       '1.0.0',
      tags:          tags ?? [],
      isPublic:      isPublic ?? false,
    }).returning();

    return NextResponse.json({ expert: inserted, message: 'Expert deployment initiated' }, { status: 201 });
  } catch (err) {
    const message = err instanceof Error ? err.message : 'Invalid request body';
    console.error('[experts POST]', err);
    return NextResponse.json({ error: message }, { status: 400 });
  }
}

/* PATCH /api/experts — Update an existing expert */
export async function PATCH(req: NextRequest) {
  try {
    const body = await req.json();
    const { id, ...updates } = body;

    if (!id) {
      return NextResponse.json({ error: 'Expert id is required' }, { status: 400 });
    }

    // Check expert exists
    const [existing] = await db.select().from(experts).where(eq(experts.id, id));
    if (!existing) {
      return NextResponse.json({ error: 'Expert not found' }, { status: 404 });
    }

    // Build update values
    const values: Record<string, unknown> = { updatedAt: new Date() };
    if (updates.name !== undefined)             values.name = updates.name.trim();
    if (updates.description !== undefined)      values.description = updates.description?.trim() || null;
    if (updates.role !== undefined)              values.role = updates.role;
    if (updates.status !== undefined)           values.status = updates.status;
    if (updates.modelId !== undefined)          values.modelId = updates.modelId;
    if (updates.modelName !== undefined)        values.modelName = updates.modelName;
    if (updates.providerId !== undefined)       values.providerId = updates.providerId;
    if (updates.providerName !== undefined)     values.providerName = updates.providerName;
    if (updates.modelSource !== undefined)      values.modelSource = updates.modelSource;
    if (updates.localModelConfig !== undefined) values.localModelConfig = updates.localModelConfig;
    if (updates.systemPrompt !== undefined)     values.systemPrompt = updates.systemPrompt?.trim() || null;
    if (updates.temperature !== undefined)      values.temperature = String(updates.temperature);
    if (updates.maxTokens !== undefined)        values.maxTokens = updates.maxTokens;
    if (updates.tags !== undefined)             values.tags = updates.tags;
    if (updates.isPublic !== undefined)         values.isPublic = updates.isPublic;
    if (updates.isFinetuned !== undefined)      values.isFinetuned = updates.isFinetuned;
    if (updates.replicaCount !== undefined)     values.replicaCount = updates.replicaCount;

    const [updated] = await db.update(experts)
      .set(values)
      .where(eq(experts.id, id))
      .returning();

    return NextResponse.json({ expert: updated });
  } catch (err) {
    console.error('[experts PATCH]', err);
    return NextResponse.json({ error: 'Update failed' }, { status: 500 });
  }
}

/* DELETE /api/experts — Remove an expert */
export async function DELETE(req: NextRequest) {
  try {
    const { searchParams } = req.nextUrl;
    const id = searchParams.get('id');

    if (!id) {
      return NextResponse.json({ error: 'Expert id is required' }, { status: 400 });
    }

    const [deleted] = await db.delete(experts).where(eq(experts.id, id)).returning();
    if (!deleted) {
      return NextResponse.json({ error: 'Expert not found' }, { status: 404 });
    }

    return NextResponse.json({ deleted: true, id });
  } catch (err) {
    console.error('[experts DELETE]', err);
    return NextResponse.json({ error: 'Delete failed' }, { status: 500 });
  }
}
