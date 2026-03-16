import { NextResponse } from 'next/server';
import { db, apiKeys } from '@/lib/db';
import { sql } from 'drizzle-orm';

export async function GET() {
  try {
    const [key] = await db
      .select({ id: apiKeys.id, keyPrefix: apiKeys.keyPrefix })
      .from(apiKeys)
      .where(sql`${apiKeys.providerId} = 'huggingface' AND ${apiKeys.status} = 'active'`)
      .limit(1);

    return NextResponse.json({
      configured: !!key,
      keyPrefix: key?.keyPrefix ?? null,
    });
  } catch {
    return NextResponse.json({ configured: false, keyPrefix: null });
  }
}
