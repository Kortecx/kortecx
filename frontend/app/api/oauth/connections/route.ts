import { NextResponse } from 'next/server';
import { db } from '@/lib/db';
import { socialConnections } from '@/lib/db/schema';
import { eq } from 'drizzle-orm';

/**
 * GET /api/oauth/connections
 *
 * List all active social connections (tokens are never exposed).
 */
export async function GET() {
  try {
    const connections = await db
      .select({
        id: socialConnections.id,
        platform: socialConnections.platform,
        platformUserId: socialConnections.platformUserId,
        platformUsername: socialConnections.platformUsername,
        platformAvatar: socialConnections.platformAvatar,
        scopes: socialConnections.scopes,
        status: socialConnections.status,
        tokenExpiresAt: socialConnections.tokenExpiresAt,
        lastUsedAt: socialConnections.lastUsedAt,
        createdAt: socialConnections.createdAt,
      })
      .from(socialConnections)
      .orderBy(socialConnections.createdAt);

    // Check for expired tokens
    const now = new Date();
    const enriched = connections.map((conn: typeof connections[number]) => ({
      ...conn,
      isExpired: conn.tokenExpiresAt ? new Date(conn.tokenExpiresAt) < now : false,
    }));

    return NextResponse.json({ connections: enriched });
  } catch (error) {
    console.error('[OAuth] Failed to list connections:', error);
    return NextResponse.json({ connections: [] });
  }
}

/**
 * DELETE /api/oauth/connections?platform=twitter
 *
 * Disconnect a social platform (removes stored tokens).
 */
export async function DELETE(request: Request) {
  const url = new URL(request.url);
  const platform = url.searchParams.get('platform');

  if (!platform) {
    return NextResponse.json({ error: 'Missing platform parameter' }, { status: 400 });
  }

  try {
    await db.delete(socialConnections).where(eq(socialConnections.platform, platform));
    return NextResponse.json({ success: true, platform, status: 'disconnected' });
  } catch (error) {
    console.error(`[OAuth] Failed to disconnect ${platform}:`, error);
    return NextResponse.json({ error: 'Failed to disconnect' }, { status: 500 });
  }
}
