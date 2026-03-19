import { NextResponse } from 'next/server';
import { cookies } from 'next/headers';
import { getPlatformConfig, getClientCredentials, getCallbackUrl } from '@/lib/oauth/platforms';
import { generateState, generateCodeVerifier, generateCodeChallenge } from '@/lib/oauth/crypto';

/**
 * GET /api/oauth/[platform]/authorize
 *
 * Initiates the OAuth 2.0 authorization flow by redirecting the user to the
 * platform's consent screen. Stores state + PKCE verifier in httpOnly cookies
 * for validation in the callback.
 */
export async function GET(
  _request: Request,
  { params }: { params: Promise<{ platform: string }> },
) {
  const { platform } = await params;

  try {
    const config = getPlatformConfig(platform);
    const { clientId } = await getClientCredentials(config);
    const callbackUrl = getCallbackUrl(platform);

    // CSRF protection
    const state = generateState();

    // Build authorize URL
    const url = new URL(config.authorizeUrl);
    url.searchParams.set('response_type', 'code');
    url.searchParams.set('client_id', clientId);
    url.searchParams.set('redirect_uri', callbackUrl);
    url.searchParams.set('state', state);
    url.searchParams.set('scope', config.scopes.join(' '));

    // PKCE: generate code_verifier + code_challenge
    let codeVerifier: string | undefined;
    if (config.usePKCE) {
      codeVerifier = generateCodeVerifier();
      const codeChallenge = await generateCodeChallenge(codeVerifier);
      url.searchParams.set('code_challenge', codeChallenge);
      url.searchParams.set('code_challenge_method', 'S256');
    }

    // Platform-specific extra params
    if (config.extraAuthorizeParams) {
      for (const [key, value] of Object.entries(config.extraAuthorizeParams)) {
        if (value) url.searchParams.set(key, value);
      }
    }

    // Store state + verifier in httpOnly cookies (short-lived, 10 min)
    const cookieStore = await cookies();
    const cookieOpts = {
      httpOnly: true,
      secure: process.env.NODE_ENV === 'production',
      sameSite: 'lax' as const,
      path: '/',
      maxAge: 600, // 10 minutes
    };

    cookieStore.set(`oauth_state_${platform}`, state, cookieOpts);
    if (codeVerifier) {
      cookieStore.set(`oauth_verifier_${platform}`, codeVerifier, cookieOpts);
    }

    return NextResponse.redirect(url.toString());
  } catch (error) {
    const message = error instanceof Error ? error.message : 'Unknown error';
    console.error(`[OAuth] Authorize error for ${platform}:`, message);

    // Redirect to connections page with error
    const errorUrl = new URL('/providers/connections', process.env.NEXT_PUBLIC_APP_URL || 'http://localhost:3000');
    errorUrl.searchParams.set('error', message);
    errorUrl.searchParams.set('platform', platform);
    return NextResponse.redirect(errorUrl.toString());
  }
}
