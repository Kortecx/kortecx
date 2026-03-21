import { NextResponse } from 'next/server';
import { createHash } from 'crypto';
import { db } from '@/lib/db';
import { oauthCredentials } from '@/lib/db/schema';
import { eq } from 'drizzle-orm';
import { encryptToken } from '@/lib/oauth/crypto';
import { logStatus } from '@/lib/status-log';

/**
 * GET /api/oauth/credentials
 *
 * List all stored OAuth app credentials (secrets are never exposed).
 * Returns platform, masked client ID, and status.
 */
export async function GET() {
  try {
    const rows: Array<{
      id: string;
      platform: string;
      clientId: string;
      status: string | null;
      updatedAt: Date;
    }> = await db
      .select({
        id: oauthCredentials.id,
        platform: oauthCredentials.platform,
        clientId: oauthCredentials.clientId,
        status: oauthCredentials.status,
        updatedAt: oauthCredentials.updatedAt,
      })
      .from(oauthCredentials);

    // Mask client IDs — show first 6 and last 4 chars
    const masked = rows.map(r => ({
      id: r.id,
      platform: r.platform,
      clientIdMasked: r.clientId.length > 10
        ? `${r.clientId.slice(0, 6)}${'*'.repeat(r.clientId.length - 10)}${r.clientId.slice(-4)}`
        : `${r.clientId.slice(0, 3)}***`,
      clientIdPrefix: r.clientId.slice(0, 6),
      status: r.status,
      updatedAt: r.updatedAt,
      hasCredentials: true,
    }));

    return NextResponse.json({ credentials: masked });
  } catch (error) {
    console.error('[OAuth] Failed to list credentials:', error);
    return NextResponse.json({ credentials: [] });
  }
}

/**
 * POST /api/oauth/credentials
 *
 * Save or update OAuth app credentials for a platform.
 * Client secret is encrypted with AES-256-GCM before storage.
 */
export async function POST(request: Request) {
  try {
    const body = await request.json();
    const { platform, clientId, clientSecret } = body;

    if (!platform || !clientId || !clientSecret) {
      return NextResponse.json(
        { error: 'Missing required fields: platform, clientId, clientSecret' },
        { status: 400 },
      );
    }

    const keyHash = createHash('sha256').update(clientId).digest('hex');
    const encryptedSecret = encryptToken(clientSecret);
    const now = new Date();

    // Check if credentials exist for this platform
    const existing: Array<{ id: string }> = await db
      .select({ id: oauthCredentials.id })
      .from(oauthCredentials)
      .where(eq(oauthCredentials.platform, platform))
      .limit(1);

    if (existing.length > 0) {
      // Update existing credentials
      await db.update(oauthCredentials)
        .set({
          clientId,
          clientSecret: encryptedSecret,
          keyHash,
          status: 'active',
          updatedAt: now,
        })
        .where(eq(oauthCredentials.platform, platform));

      logStatus('info', `OAuth credentials updated for ${platform}`, 'oauth', { platform });
      return NextResponse.json({
        success: true,
        action: 'updated',
        platform,
        clientIdMasked: clientId.length > 10
          ? `${clientId.slice(0, 6)}${'*'.repeat(clientId.length - 10)}${clientId.slice(-4)}`
          : `${clientId.slice(0, 3)}***`,
      });
    }

    // Insert new credentials
    await db.insert(oauthCredentials).values({
      id: `oc-${platform}-${Date.now()}`,
      platform,
      clientId,
      clientSecret: encryptedSecret,
      keyHash,
      status: 'active',
      createdAt: now,
      updatedAt: now,
    });

    logStatus('info', `OAuth credentials saved for ${platform}`, 'oauth', { platform });
    return NextResponse.json({
      success: true,
      action: 'created',
      platform,
      clientIdMasked: clientId.length > 10
        ? `${clientId.slice(0, 6)}${'*'.repeat(clientId.length - 10)}${clientId.slice(-4)}`
        : `${clientId.slice(0, 3)}***`,
    });

  } catch (error) {
    const message = error instanceof Error ? error.message : 'Unknown error';
    console.error('[OAuth] Failed to save credentials:', message);
    logStatus('error', `OAuth credentials save failed: ${message}`, 'oauth', { error: message });
    return NextResponse.json({ error: message }, { status: 500 });
  }
}

/**
 * DELETE /api/oauth/credentials?platform=twitter
 *
 * Remove stored OAuth credentials for a platform.
 */
export async function DELETE(request: Request) {
  const url = new URL(request.url);
  const platform = url.searchParams.get('platform');

  if (!platform) {
    return NextResponse.json({ error: 'Missing platform parameter' }, { status: 400 });
  }

  try {
    await db.delete(oauthCredentials).where(eq(oauthCredentials.platform, platform));
    logStatus('info', `OAuth credentials removed for ${platform}`, 'oauth', { platform });
    return NextResponse.json({ success: true, platform });
  } catch (error) {
    console.error(`[OAuth] Failed to delete credentials for ${platform}:`, error);
    logStatus('error', `OAuth credentials removal failed for ${platform}`, 'oauth', { platform, error: error instanceof Error ? error.message : 'Unknown' });
    return NextResponse.json({ error: 'Failed to delete credentials' }, { status: 500 });
  }
}
