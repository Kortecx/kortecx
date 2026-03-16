import { NextRequest, NextResponse } from 'next/server';
import { db, datasetSchemas, datasets } from '@/lib/db';
import { eq, desc } from 'drizzle-orm';

/* GET /api/schemas?datasetId=<id> — get schema for a dataset */
export async function GET(req: NextRequest) {
  const datasetId = req.nextUrl.searchParams.get('datasetId');
  const id = req.nextUrl.searchParams.get('id');

  try {
    if (id) {
      const [row] = await db.select().from(datasetSchemas).where(eq(datasetSchemas.id, id));
      if (!row) return NextResponse.json({ error: 'Not found' }, { status: 404 });
      return NextResponse.json({ schema: row });
    }

    if (datasetId) {
      const rows = await db.select().from(datasetSchemas)
        .where(eq(datasetSchemas.datasetId, datasetId))
        .orderBy(desc(datasetSchemas.version));
      return NextResponse.json({ schemas: rows, total: rows.length });
    }

    // All schemas (templates + dataset-linked)
    const rows = await db.select().from(datasetSchemas).orderBy(desc(datasetSchemas.updatedAt));
    return NextResponse.json({ schemas: rows, total: rows.length });
  } catch (err) {
    console.error('[schemas GET]', err);
    return NextResponse.json({ schemas: [], total: 0 });
  }
}

/* POST /api/schemas — create a schema definition */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { datasetId, name, columns, isTemplate } = body;

    if (!name?.trim() || !columns || !Array.isArray(columns)) {
      return NextResponse.json({ error: 'name and columns array required' }, { status: 400 });
    }

    const id = `schema-${Date.now()}`;
    const [inserted] = await db.insert(datasetSchemas).values({
      id,
      datasetId: datasetId ?? null,
      name: name.trim(),
      columns,
      isTemplate: isTemplate ?? false,
    }).returning();

    // Link to dataset if provided
    if (datasetId) {
      await db.update(datasets).set({ schemaId: id, updatedAt: new Date() }).where(eq(datasets.id, datasetId));
    }

    return NextResponse.json({ schema: inserted }, { status: 201 });
  } catch (err) {
    console.error('[schemas POST]', err);
    return NextResponse.json({ error: 'Failed to create schema' }, { status: 500 });
  }
}

/* PATCH /api/schemas — update schema (bumps version) */
export async function PATCH(req: NextRequest) {
  try {
    const body = await req.json();
    const { id, columns, name } = body;
    if (!id) return NextResponse.json({ error: 'id required' }, { status: 400 });

    const [existing] = await db.select().from(datasetSchemas).where(eq(datasetSchemas.id, id));
    if (!existing) return NextResponse.json({ error: 'Not found' }, { status: 404 });

    const values: Record<string, unknown> = {
      updatedAt: new Date(),
      version: (existing.version ?? 1) + 1,
    };
    if (columns) values.columns = columns;
    if (name) values.name = name.trim();

    const [updated] = await db.update(datasetSchemas).set(values).where(eq(datasetSchemas.id, id)).returning();
    return NextResponse.json({ schema: updated });
  } catch (err) {
    console.error('[schemas PATCH]', err);
    return NextResponse.json({ error: 'Update failed' }, { status: 500 });
  }
}
