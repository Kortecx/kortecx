import { NextResponse } from 'next/server';
import { db } from '@/lib/db';
import { socialConnections } from '@/lib/db/schema';
import { eq } from 'drizzle-orm';
import { decryptToken, encryptToken } from '@/lib/oauth/crypto';
import { getPlatformConfig, getClientCredentials } from '@/lib/oauth/platforms';
import { logStatus } from '@/lib/status-log';

export interface PlatformPermissions {
  consume: boolean;
  generate: boolean;
  publish: boolean;
  schedule: boolean;
  report: boolean;
  execute: boolean;
}

const DEFAULT_PERMISSIONS: PlatformPermissions = {
  consume: true, generate: true, publish: true, schedule: true, report: true, execute: true,
};

/**
 * GET /api/oauth/connections/configure?platform=linkedin
 *
 * Get full connection details for configuring a platform:
 * token status, permissions, scopes, profile info, credential status.
 */
export async function GET(request: Request) {
  const url = new URL(request.url);
  const platform = url.searchParams.get('platform');

  if (!platform) {
    return NextResponse.json({ error: 'Missing platform parameter' }, { status: 400 });
  }

  try {
    const rows: Array<{
      id: string;
      platform: string;
      tokenExpiresAt: Date | null;
      scopes: string[] | null;
      platformUserId: string | null;
      platformUsername: string | null;
      platformAvatar: string | null;
      permissions: unknown;
      status: string | null;
      refreshToken: string | null;
      lastUsedAt: Date | null;
      lastRefreshedAt: Date | null;
      createdAt: Date;
      updatedAt: Date;
    }> = await db
      .select({
        id: socialConnections.id,
        platform: socialConnections.platform,
        tokenExpiresAt: socialConnections.tokenExpiresAt,
        scopes: socialConnections.scopes,
        platformUserId: socialConnections.platformUserId,
        platformUsername: socialConnections.platformUsername,
        platformAvatar: socialConnections.platformAvatar,
        permissions: socialConnections.permissions,
        status: socialConnections.status,
        refreshToken: socialConnections.refreshToken,
        lastUsedAt: socialConnections.lastUsedAt,
        lastRefreshedAt: socialConnections.lastRefreshedAt,
        createdAt: socialConnections.createdAt,
        updatedAt: socialConnections.updatedAt,
      })
      .from(socialConnections)
      .where(eq(socialConnections.platform, platform))
      .limit(1);

    if (rows.length === 0) {
      return NextResponse.json({ error: 'No connection found for this platform' }, { status: 404 });
    }

    const conn = rows[0];
    const now = new Date();
    const isExpired = conn.tokenExpiresAt ? new Date(conn.tokenExpiresAt) < now : false;
    const hasRefreshToken = !!conn.refreshToken;
    const permissions = (conn.permissions as PlatformPermissions) || DEFAULT_PERMISSIONS;

    return NextResponse.json({
      connection: {
        id: conn.id,
        platform: conn.platform,
        platformUserId: conn.platformUserId,
        platformUsername: conn.platformUsername,
        platformAvatar: conn.platformAvatar,
        scopes: conn.scopes || [],
        permissions,
        status: isExpired ? 'expired' : conn.status,
        isExpired,
        hasRefreshToken,
        tokenExpiresAt: conn.tokenExpiresAt,
        lastUsedAt: conn.lastUsedAt,
        lastRefreshedAt: conn.lastRefreshedAt,
        connectedAt: conn.createdAt,
        updatedAt: conn.updatedAt,
      },
    });
  } catch (error) {
    console.error(`[OAuth] Failed to get config for ${platform}:`, error);
    return NextResponse.json({ error: 'Failed to load connection' }, { status: 500 });
  }
}

/**
 * PUT /api/oauth/connections/configure
 *
 * Update connection settings: permissions, or trigger a token refresh.
 */
export async function PUT(request: Request) {
  try {
    const body = await request.json();
    const { platform, action, permissions } = body;

    if (!platform) {
      return NextResponse.json({ error: 'Missing platform' }, { status: 400 });
    }

    // Action: update permissions
    if (action === 'update_permissions' && permissions) {
      await db.update(socialConnections)
        .set({
          permissions: permissions as PlatformPermissions,
          updatedAt: new Date(),
        })
        .where(eq(socialConnections.platform, platform));

      logStatus('info', `OAuth connection configured: ${platform}`, 'oauth', { platform, action: 'update_permissions' });
      return NextResponse.json({ success: true, action: 'permissions_updated', permissions });
    }

    // Action: refresh token
    if (action === 'refresh_token') {
      const connRows: Array<{
        id: string;
        refreshToken: string | null;
        status: string | null;
      }> = await db
        .select({
          id: socialConnections.id,
          refreshToken: socialConnections.refreshToken,
          status: socialConnections.status,
        })
        .from(socialConnections)
        .where(eq(socialConnections.platform, platform))
        .limit(1);

      if (connRows.length === 0) {
        return NextResponse.json({ error: 'No connection found' }, { status: 404 });
      }

      const conn = connRows[0];
      if (!conn.refreshToken) {
        return NextResponse.json({ error: 'No refresh token available. Please reconnect.' }, { status: 400 });
      }

      const config = getPlatformConfig(platform);
      const { clientId, clientSecret } = await getClientCredentials(config);
      const decryptedRefresh = decryptToken(conn.refreshToken);

      const refreshParams = new URLSearchParams();
      refreshParams.set('grant_type', 'refresh_token');
      refreshParams.set('refresh_token', decryptedRefresh);

      const headers: Record<string, string> = {
        'Content-Type': 'application/x-www-form-urlencoded',
        'Accept': 'application/json',
      };

      if (config.tokenAuthMethod === 'basic') {
        headers['Authorization'] = 'Basic ' + Buffer.from(`${clientId}:${clientSecret}`).toString('base64');
      } else {
        refreshParams.set('client_id', clientId);
        refreshParams.set('client_secret', clientSecret);
      }

      const refreshRes = await fetch(config.tokenUrl, {
        method: 'POST', headers, body: refreshParams.toString(),
      });

      if (!refreshRes.ok) {
        const errBody = await refreshRes.text();
        console.error(`[OAuth] Token refresh failed for ${platform}:`, refreshRes.status, errBody);
        await db.update(socialConnections)
          .set({ status: 'expired', updatedAt: new Date() })
          .where(eq(socialConnections.id, conn.id));
        return NextResponse.json({ error: 'Token refresh failed. Please reconnect.' }, { status: 400 });
      }

      const data = await refreshRes.json();
      const now = new Date();
      const tokenExpiresAt = data.expires_in ? new Date(Date.now() + data.expires_in * 1000) : null;

      await db.update(socialConnections)
        .set({
          accessToken: encryptToken(data.access_token),
          refreshToken: data.refresh_token ? encryptToken(data.refresh_token) : conn.refreshToken,
          tokenExpiresAt,
          status: 'active',
          lastRefreshedAt: now,
          updatedAt: now,
        })
        .where(eq(socialConnections.id, conn.id));

      logStatus('info', `OAuth connection configured: ${platform}`, 'oauth', { platform, action: 'refresh_token' });
      return NextResponse.json({
        success: true,
        action: 'token_refreshed',
        tokenExpiresAt,
        status: 'active',
      });
    }

    return NextResponse.json({ error: 'Invalid action' }, { status: 400 });
  } catch (error) {
    const message = error instanceof Error ? error.message : 'Unknown error';
    console.error('[OAuth] Configure error:', message);
    logStatus('error', `OAuth configure failed: ${message}`, 'oauth', { error: message });
    return NextResponse.json({ error: message }, { status: 500 });
  }
}
