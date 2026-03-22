import { NextResponse } from 'next/server';
import { cookies } from 'next/headers';
import { db } from '@/lib/db';
import { socialConnections } from '@/lib/db/schema';
import { eq } from 'drizzle-orm';
import { getPlatformConfig, getClientCredentials, getCallbackUrl, resolvePath } from '@/lib/oauth/platforms';
import { encryptToken } from '@/lib/oauth/crypto';
import { logStatus } from '@/lib/status-log';

/**
 * GET /api/oauth/[platform]/callback
 *
 * OAuth 2.0 callback handler. Exchanges the authorization code for tokens,
 * fetches the user's profile, encrypts tokens, and stores the connection.
 */
export async function GET(
  request: Request,
  { params }: { params: Promise<{ platform: string }> },
) {
  const { platform } = await params;
  const url = new URL(request.url);
  const code = url.searchParams.get('code');
  const state = url.searchParams.get('state');
  const error = url.searchParams.get('error');
  const errorDescription = url.searchParams.get('error_description');

  const appBase = process.env.NEXT_PUBLIC_APP_URL || 'http://localhost:3000';
  const redirectUrl = new URL('/providers/connections', appBase);

  // Handle OAuth errors from the provider
  if (error) {
    console.error(`[OAuth] Provider error for ${platform}:`, error, errorDescription);
    redirectUrl.searchParams.set('error', errorDescription || error);
    redirectUrl.searchParams.set('platform', platform);
    return NextResponse.redirect(redirectUrl.toString());
  }

  if (!code || !state) {
    redirectUrl.searchParams.set('error', 'Missing authorization code or state');
    redirectUrl.searchParams.set('platform', platform);
    return NextResponse.redirect(redirectUrl.toString());
  }

  try {
    const config = getPlatformConfig(platform);
    const { clientId, clientSecret } = await getClientCredentials(config);
    const callbackUrl = getCallbackUrl(platform);

    // Validate CSRF state
    const cookieStore = await cookies();
    const savedState = cookieStore.get(`oauth_state_${platform}`)?.value;
    if (!savedState || savedState !== state) {
      throw new Error('Invalid OAuth state — possible CSRF attack. Please try again.');
    }

    // Clean up state cookie
    cookieStore.delete(`oauth_state_${platform}`);

    // Build token exchange request
    const tokenParams = new URLSearchParams();
    tokenParams.set('grant_type', 'authorization_code');
    tokenParams.set('code', code);
    tokenParams.set('redirect_uri', callbackUrl);

    // PKCE: include code_verifier if this platform uses it
    if (config.usePKCE) {
      const verifier = cookieStore.get(`oauth_verifier_${platform}`)?.value;
      if (!verifier) throw new Error('Missing PKCE code verifier. Please try again.');
      tokenParams.set('code_verifier', verifier);
      cookieStore.delete(`oauth_verifier_${platform}`);
    }

    // Auth method: body params vs Basic auth header
    const headers: Record<string, string> = {
      'Content-Type': 'application/x-www-form-urlencoded',
      'Accept': 'application/json',
    };

    if (config.tokenAuthMethod === 'basic') {
      headers['Authorization'] = 'Basic ' + Buffer.from(`${clientId}:${clientSecret}`).toString('base64');
    } else {
      tokenParams.set('client_id', clientId);
      tokenParams.set('client_secret', clientSecret);
    }

    // Extra token params
    if (config.extraTokenParams) {
      for (const [key, value] of Object.entries(config.extraTokenParams)) {
        tokenParams.set(key, value);
      }
    }

    // Exchange code for tokens
    const tokenRes = await fetch(config.tokenUrl, {
      method: 'POST',
      headers,
      body: tokenParams.toString(),
    });

    if (!tokenRes.ok) {
      const errBody = await tokenRes.text();
      console.error(`[OAuth] Token exchange failed for ${platform}:`, tokenRes.status, errBody);
      throw new Error(`Token exchange failed (${tokenRes.status}). Check your OAuth app configuration.`);
    }

    const tokenData = await tokenRes.json();
    const accessToken: string = tokenData.access_token;
    const refreshToken: string | undefined = tokenData.refresh_token;
    const expiresIn: number | undefined = tokenData.expires_in;

    if (!accessToken) {
      throw new Error('No access_token in token response');
    }

    // Fetch user profile
    let profileData: Record<string, unknown> = {};
    try {
      const profileRes = await fetch(config.profileUrl, {
        headers: { 'Authorization': `Bearer ${accessToken}`, 'Accept': 'application/json' },
      });
      if (profileRes.ok) {
        profileData = await profileRes.json();
      }
    } catch (profileErr) {
      console.warn(`[OAuth] Could not fetch profile for ${platform}:`, profileErr);
    }

    // Extract user info
    const platformUserId = resolvePath(profileData, config.profileMapping.id) || '';
    const platformUsername = resolvePath(profileData, config.profileMapping.username) || platform;
    const platformAvatar = config.profileMapping.avatar
      ? resolvePath(profileData, config.profileMapping.avatar)
      : undefined;

    // Encrypt tokens
    const encryptedAccess = encryptToken(accessToken);
    const encryptedRefresh = refreshToken ? encryptToken(refreshToken) : null;

    // Calculate token expiry
    const tokenExpiresAt = expiresIn
      ? new Date(Date.now() + expiresIn * 1000)
      : null;

    // Upsert connection — replace existing connection for this platform
    const connectionId = `sc-${platform}-${Date.now()}`;
    const now = new Date();

    // Delete existing connection for this platform (only one active connection per platform)
    await db.delete(socialConnections).where(eq(socialConnections.platform, platform));

    // Insert new connection
    await db.insert(socialConnections).values({
      id: connectionId,
      platform,
      accessToken: encryptedAccess,
      refreshToken: encryptedRefresh,
      tokenExpiresAt,
      scopes: config.scopes,
      platformUserId,
      platformUsername,
      platformAvatar: platformAvatar || null,
      platformMeta: profileData,
      status: 'active',
      lastUsedAt: now,
      lastRefreshedAt: now,
      createdAt: now,
      updatedAt: now,
    });

    logStatus('info', `OAuth connected: ${platform}`, 'oauth', { platform, username: platformUsername });

    // Success — notify opener window and close popup
    return new NextResponse(popupResultPage('oauth_success', {
      platform,
      username: platformUsername,
    }), { headers: { 'Content-Type': 'text/html' } });

  } catch (err) {
    const message = err instanceof Error ? err.message : 'Unknown error';
    console.error(`[OAuth] Callback error for ${platform}:`, message);
    logStatus('error', `OAuth callback failed for ${platform}: ${message}`, 'oauth', { platform, error: message });

    return new NextResponse(popupResultPage('oauth_error', {
      platform,
      error: message,
    }), { headers: { 'Content-Type': 'text/html' } });
  }
}

/**
 * Returns an HTML page that posts a message to the opener window and closes
 * itself. Falls back to redirect if not in a popup.
 */
function popupResultPage(type: 'oauth_success' | 'oauth_error', data: Record<string, string>): string {
  const payload = JSON.stringify({ type, ...data });
  const fallbackUrl = `/providers/connections?${type === 'oauth_success' ? `connected=${data.platform}&username=${data.username || ''}` : `error=${encodeURIComponent(data.error || 'Unknown error')}&platform=${data.platform}`}`;

  return `<!DOCTYPE html>
<html><head><title>Connecting...</title></head>
<body>
<p style="font-family:system-ui;text-align:center;margin-top:40vh;color:#666">
  ${type === 'oauth_success' ? 'Connected successfully. This window will close.' : 'Connection failed. This window will close.'}
</p>
<script>
  (function() {
    try {
      if (window.opener) {
        window.opener.postMessage(${payload}, window.location.origin);
        window.close();
      } else {
        window.location.href = ${JSON.stringify(fallbackUrl)};
      }
    } catch(e) {
      window.location.href = ${JSON.stringify(fallbackUrl)};
    }
  })();
</script>
</body></html>`;
}
