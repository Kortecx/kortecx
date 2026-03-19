import { NextResponse } from 'next/server';
import { db } from '@/lib/db';
import { socialConnections } from '@/lib/db/schema';
import { eq } from 'drizzle-orm';
import { PLATFORMS } from '@/lib/constants';

/**
 * GET /api/platforms
 *
 * Lists all supported platforms with their connection status from the database.
 */
export async function GET() {
  try {
    // Get all active connections from DB
    const connections: Array<{
      platform: string;
      platformUsername: string | null;
      platformAvatar: string | null;
      status: string | null;
    }> = await db
      .select({
        platform: socialConnections.platform,
        platformUsername: socialConnections.platformUsername,
        platformAvatar: socialConnections.platformAvatar,
        status: socialConnections.status,
      })
      .from(socialConnections);

    const connMap = new Map(connections.map(c => [c.platform, c]));

    // Merge with platform catalog
    const platforms = PLATFORMS.map(p => {
      const conn = connMap.get(p.id);
      return {
        id: p.id,
        name: p.name,
        color: p.color,
        connected: conn?.status === 'active',
        username: conn?.platformUsername || undefined,
        avatar: conn?.platformAvatar || undefined,
        status: conn?.status || 'disconnected',
      };
    });

    return NextResponse.json({ platforms });
  } catch {
    // Fallback to static list if DB unavailable
    const platforms = PLATFORMS.map(p => ({
      id: p.id,
      name: p.name,
      color: p.color,
      connected: false,
      status: 'disconnected',
    }));
    return NextResponse.json({ platforms });
  }
}

/**
 * POST /api/platforms
 *
 * Connect/disconnect a platform. For OAuth platforms, this returns the
 * OAuth authorization URL for the frontend to redirect to.
 */
export async function POST(request: Request) {
  const body = await request.json();
  const { platformId, action } = body;

  if (action === 'connect') {
    // Return the OAuth authorize URL for the frontend to redirect to
    const appBase = process.env.NEXT_PUBLIC_APP_URL || 'http://localhost:3000';
    const authorizeUrl = `${appBase}/api/oauth/${platformId}/authorize`;
    return NextResponse.json({
      success: true,
      platformId,
      action: 'connect',
      authorizeUrl,
    });
  }

  if (action === 'disconnect') {
    try {
      await db.delete(socialConnections).where(eq(socialConnections.platform, platformId));
    } catch (error) {
      console.error(`[Platforms] Failed to disconnect ${platformId}:`, error);
    }
    return NextResponse.json({
      success: true,
      platformId,
      action: 'disconnect',
      status: 'disconnected',
    });
  }

  return NextResponse.json({ error: 'Invalid action' }, { status: 400 });
}
