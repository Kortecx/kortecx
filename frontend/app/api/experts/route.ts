import { NextRequest, NextResponse } from 'next/server';
import { db, experts, expertRuns } from '@/lib/db';
import { eq, ilike, or, desc, asc, sql } from 'drizzle-orm';
import { logStatus } from '@/lib/status-log';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/** Fire-and-forget: embed an agent into Qdrant for graph similarity. */
function embedAgent(expertId: string): void {
  fetch(`${ENGINE_URL}/api/agents/engine/${expertId}/embed`, { method: 'POST' }).catch((err) => {
    console.warn('[experts] embed failed:', err);
  });
}

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

/* POST /api/experts — Create a new expert via engine (local files first, then synced to DB) */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const {
      name, role, modelId, providerId, systemPrompt, temperature,
      maxTokens, description, tags, isPublic, modelSource, localModelConfig,
      category, complexityLevel, capabilities, customRoleDescription, specializations,
    } = body;

    if (!name?.trim()) {
      return NextResponse.json({ error: 'Expert name is required' }, { status: 400 });
    }
    if (!role) {
      return NextResponse.json({ error: 'Expert role is required' }, { status: 400 });
    }

    // Route creation through the engine — creates local files on disk,
    // syncs to NeonDB, and auto-embeds into Qdrant for the graph.
    const engineRes = await fetch(`${ENGINE_URL}/api/agents/engine/create`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        name: name.trim(),
        role,
        description: description?.trim() || '',
        systemPrompt: systemPrompt?.trim() || '',
        modelSource: modelSource || 'local',
        localModelConfig: localModelConfig || { engine: 'ollama', modelName: 'llama3.2:3b' },
        temperature: temperature ?? 0.7,
        maxTokens: maxTokens ?? 4096,
        tags: tags ?? [],
        isPublic: isPublic ?? false,
        category: category ?? 'custom',
        complexityLevel: complexityLevel ?? 3,
        capabilities: capabilities ?? [],
        customRoleDescription: customRoleDescription ?? '',
        specializations: specializations ?? [],
      }),
    });

    if (!engineRes.ok) {
      const errText = await engineRes.text().catch(() => 'Engine error');
      throw new Error(`Engine create failed (${engineRes.status}): ${errText}`);
    }

    const engineData = await engineRes.json();
    const expert = engineData.expert || engineData;

    // Insert into NeonDB immediately so agents page sees it right away
    // (engine's async sync may not finish before the page redirects)
    try {
      await db.insert(experts).values({
        id: expert.id,
        name: name.trim(),
        description: description?.trim() || null,
        role,
        status: 'idle',
        version: '1.0.0',
        modelId: body.modelId || localModelConfig?.model || localModelConfig?.modelName || 'llama3.2:3b',
        modelName: body.modelId || localModelConfig?.model || localModelConfig?.modelName || 'llama3.2:3b',
        providerId: body.providerId || localModelConfig?.engine || 'ollama',
        providerName: body.providerId || localModelConfig?.engine || 'ollama',
        modelSource: modelSource || 'local',
        localModelConfig: localModelConfig || null,
        systemPrompt: systemPrompt?.trim() || null,
        temperature: String(temperature ?? 0.7),
        maxTokens: maxTokens ?? 4096,
        tags: tags ?? [],
        isPublic: isPublic ?? false,
        category: category ?? 'custom',
        complexityLevel: complexityLevel ?? 3,
      }).onConflictDoNothing();
    } catch (dbErr) {
      console.warn('[experts POST] direct DB insert failed (engine sync will retry):', dbErr);
    }

    logStatus('info', `Expert deployed: ${name}`, 'expert', { id: expert.id, role, modelSource: modelSource || 'local' });
    return NextResponse.json({ expert, message: 'Expert deployment initiated' }, { status: 201 });
  } catch (err) {
    const message = err instanceof Error ? err.message : 'Invalid request body';
    console.error('[experts POST]', err);
    logStatus('error', `Expert creation failed: ${message}`, 'expert', { error: message });
    return NextResponse.json({ error: message }, { status: 400 });
  }
}

/* PATCH /api/experts — Update an existing expert (syncs to engine local files + NeonDB) */
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

    // Sync file-backed fields to engine local files (non-blocking)
    const fileFields = ['name', 'description', 'role', 'temperature', 'maxTokens', 'tags',
      'isPublic', 'category', 'complexityLevel', 'modelSource', 'localModelConfig'];
    if (fileFields.some(f => updates[f] !== undefined)) {
      // Update expert.json on disk via engine
      const merged = { ...existing, ...updates, updatedAt: new Date().toISOString() };
      fetch(`${ENGINE_URL}/api/agents/engine/${id}/update`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          filename: 'expert.json',
          content: JSON.stringify({
            id: merged.id,
            name: merged.name,
            description: merged.description,
            role: merged.role,
            version: merged.version,
            modelSource: merged.modelSource,
            localModelConfig: merged.localModelConfig,
            temperature: merged.temperature,
            maxTokens: merged.maxTokens,
            tags: merged.tags,
            isPublic: merged.isPublic,
            category: merged.category,
            complexityLevel: merged.complexityLevel,
            createdAt: merged.createdAt,
            updatedAt: new Date().toISOString(),
          }, null, 2),
        }),
      }).catch((err) => console.warn('[experts PATCH] engine expert.json sync failed:', err));
    }

    // Update system.md on disk if systemPrompt changed
    if (updates.systemPrompt !== undefined) {
      fetch(`${ENGINE_URL}/api/agents/engine/${id}/update`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ filename: 'system.md', content: updates.systemPrompt || '' }),
      }).catch((err) => console.warn('[experts PATCH] engine system.md sync failed:', err));
    }

    // Build update values for NeonDB
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
    if (updates.category !== undefined)         values.category = updates.category;
    if (updates.complexityLevel !== undefined)  values.complexityLevel = updates.complexityLevel;
    if (updates.isFinetuned !== undefined)      values.isFinetuned = updates.isFinetuned;
    if (updates.replicaCount !== undefined)     values.replicaCount = updates.replicaCount;

    const [updated] = await db.update(experts)
      .set(values)
      .where(eq(experts.id, id))
      .returning();

    // Re-embed into Qdrant if any graph-relevant field changed (non-blocking)
    const graphFields = ['name', 'description', 'role', 'category', 'tags', 'complexityLevel', 'systemPrompt'];
    if (graphFields.some(f => updates[f] !== undefined)) {
      embedAgent(id);
    }

    logStatus('info', `Expert updated: ${id}`, 'expert', { id, fields: Object.keys(updates) });
    return NextResponse.json({ expert: updated });
  } catch (err) {
    console.error('[experts PATCH]', err);
    logStatus('error', `Expert update failed: ${err instanceof Error ? err.message : 'Unknown'}`, 'expert', { error: err instanceof Error ? err.message : 'Unknown' });
    return NextResponse.json({ error: 'Update failed' }, { status: 500 });
  }
}

/* DELETE /api/experts — Remove an expert (also cleans Qdrant via engine) */
export async function DELETE(req: NextRequest) {
  try {
    const { searchParams } = req.nextUrl;
    const id = searchParams.get('id');

    if (!id) {
      return NextResponse.json({ error: 'Expert id is required' }, { status: 400 });
    }

    // Delete expert from database
    const [deleted] = await db.delete(experts).where(eq(experts.id, id)).returning();
    if (!deleted) {
      return NextResponse.json({ error: 'Expert not found' }, { status: 404 });
    }

    // Delete related expert runs from database
    try {
      await db.delete(expertRuns).where(eq(expertRuns.expertId, id));
    } catch (runErr) {
      console.warn('[experts DELETE] failed to clean expert runs:', runErr);
    }

    // Delete from engine local directory (non-blocking)
    fetch(`${ENGINE_URL}/api/agents/engine/${id}`, { method: 'DELETE' }).catch((err) => {
      console.warn('[experts DELETE] engine cleanup failed:', err);
    });

    logStatus('info', `Expert deleted: ${id}`, 'expert', { id });
    return NextResponse.json({ deleted: true, id });
  } catch (err) {
    console.error('[experts DELETE]', err);
    logStatus('error', `Expert deletion failed: ${err instanceof Error ? err.message : 'Unknown'}`, 'expert', { error: err instanceof Error ? err.message : 'Unknown' });
    return NextResponse.json({ error: 'Delete failed' }, { status: 500 });
  }
}
