/**
 * Token Refresh — automatically refreshes expired OAuth tokens.
 *
 * Call getValidAccessToken(platform) before any platform API call.
 * It returns a decrypted, valid access token — refreshing if needed.
 */

import { db } from '@/lib/db';
import { socialConnections } from '@/lib/db/schema';
import { eq } from 'drizzle-orm';
import { encryptToken, decryptToken } from './crypto';
import { getPlatformConfig, getClientCredentials } from './platforms';

export interface ValidToken {
  accessToken: string;
  platformUserId: string;
  platformUsername: string;
  connectionId: string;
}

/**
 * Get a valid (decrypted) access token for a platform.
 * Automatically refreshes if expired and refresh_token is available.
 * Throws if no connection exists or refresh fails.
 */
export async function getValidAccessToken(platform: string): Promise<ValidToken> {
  const [conn] = await db
    .select()
    .from(socialConnections)
    .where(eq(socialConnections.platform, platform))
    .limit(1);

  if (!conn) {
    throw new Error(`No ${platform} connection found. Please connect your account first.`);
  }

  if (conn.status === 'revoked') {
    throw new Error(`${platform} connection has been revoked. Please reconnect.`);
  }

  const now = new Date();
  const isExpired = conn.tokenExpiresAt && new Date(conn.tokenExpiresAt) < now;

  // Token is still valid — return it
  if (!isExpired) {
    return {
      accessToken: decryptToken(conn.accessToken),
      platformUserId: conn.platformUserId || '',
      platformUsername: conn.platformUsername || '',
      connectionId: conn.id,
    };
  }

  // Token expired — attempt refresh
  if (!conn.refreshToken) {
    // Mark as expired and throw
    await db.update(socialConnections)
      .set({ status: 'expired', updatedAt: now })
      .where(eq(socialConnections.id, conn.id));
    throw new Error(`${platform} token expired and no refresh token available. Please reconnect.`);
  }

  // Refresh the token
  const config = getPlatformConfig(platform);
  const { clientId, clientSecret } = await getClientCredentials(config);

  const refreshParams = new URLSearchParams();
  refreshParams.set('grant_type', 'refresh_token');
  refreshParams.set('refresh_token', decryptToken(conn.refreshToken));

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
    method: 'POST',
    headers,
    body: refreshParams.toString(),
  });

  if (!refreshRes.ok) {
    const errBody = await refreshRes.text();
    console.error(`[OAuth] Token refresh failed for ${platform}:`, refreshRes.status, errBody);
    await db.update(socialConnections)
      .set({ status: 'expired', updatedAt: now })
      .where(eq(socialConnections.id, conn.id));
    throw new Error(`Failed to refresh ${platform} token. Please reconnect your account.`);
  }

  const refreshData = await refreshRes.json();
  const newAccessToken: string = refreshData.access_token;
  const newRefreshToken: string | undefined = refreshData.refresh_token;
  const expiresIn: number | undefined = refreshData.expires_in;

  // Update stored tokens
  const tokenExpiresAt = expiresIn ? new Date(Date.now() + expiresIn * 1000) : null;
  await db.update(socialConnections)
    .set({
      accessToken: encryptToken(newAccessToken),
      refreshToken: newRefreshToken ? encryptToken(newRefreshToken) : conn.refreshToken,
      tokenExpiresAt,
      status: 'active',
      lastRefreshedAt: now,
      updatedAt: now,
    })
    .where(eq(socialConnections.id, conn.id));

  return {
    accessToken: newAccessToken,
    platformUserId: conn.platformUserId || '',
    platformUsername: conn.platformUsername || '',
    connectionId: conn.id,
  };
}
