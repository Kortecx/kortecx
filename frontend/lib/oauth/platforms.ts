/**
 * OAuth Platform Registry — endpoints, scopes, and configuration for each
 * social media platform's OAuth 2.0 flow.
 *
 * Each platform uses confidential OAuth 2.0 (client_id + client_secret)
 * with PKCE where supported, to authenticate as a real user — not a bot.
 */

export interface OAuthPlatformConfig {
  /** Platform identifier (matches socialConnections.platform). */
  id: string;
  /** Human-readable name. */
  name: string;
  /** OAuth 2.0 authorization URL. */
  authorizeUrl: string;
  /** OAuth 2.0 token exchange URL. */
  tokenUrl: string;
  /** Scopes required for consume + publish + analytics. */
  scopes: string[];
  /** Env var name for client ID. */
  clientIdEnv: string;
  /** Env var name for client secret. */
  clientSecretEnv: string;
  /** Whether this platform requires PKCE. */
  usePKCE: boolean;
  /** API base URL for platform operations. */
  apiBaseUrl: string;
  /** URL to fetch user profile after authorization. */
  profileUrl: string;
  /** How to extract user info from profile response. */
  profileMapping: {
    id: string;       // JSONPath-like dot path to user ID
    username: string;  // dot path to username/handle
    avatar?: string;   // dot path to avatar URL
    name?: string;     // dot path to display name
  };
  /** Token response format quirks. */
  tokenResponseType?: 'json' | 'form';
  /** Extra authorize URL params. */
  extraAuthorizeParams?: Record<string, string>;
  /** Extra token request params. */
  extraTokenParams?: Record<string, string>;
  /** Custom auth header for token exchange (e.g., Reddit uses Basic auth). */
  tokenAuthMethod?: 'body' | 'basic';
}

export const OAUTH_PLATFORMS: Record<string, OAuthPlatformConfig> = {
  twitter: {
    id: 'twitter',
    name: 'X (Twitter)',
    authorizeUrl: 'https://twitter.com/i/oauth2/authorize',
    tokenUrl: 'https://api.twitter.com/2/oauth2/token',
    scopes: ['tweet.read', 'tweet.write', 'users.read', 'offline.access', 'like.read', 'like.write', 'list.read', 'bookmark.read'],
    clientIdEnv: 'TWITTER_CLIENT_ID',
    clientSecretEnv: 'TWITTER_CLIENT_SECRET',
    usePKCE: true,
    apiBaseUrl: 'https://api.twitter.com/2',
    profileUrl: 'https://api.twitter.com/2/users/me?user.fields=profile_image_url,public_metrics,description',
    profileMapping: { id: 'data.id', username: 'data.username', avatar: 'data.profile_image_url', name: 'data.name' },
    extraAuthorizeParams: { 'code_challenge_method': 'S256' },
  },

  linkedin: {
    id: 'linkedin',
    name: 'LinkedIn',
    authorizeUrl: 'https://www.linkedin.com/oauth/v2/authorization',
    tokenUrl: 'https://www.linkedin.com/oauth/v2/accessToken',
    scopes: ['openid', 'profile', 'email', 'w_member_social'],
    clientIdEnv: 'LINKEDIN_CLIENT_ID',
    clientSecretEnv: 'LINKEDIN_CLIENT_SECRET',
    usePKCE: false,
    apiBaseUrl: 'https://api.linkedin.com/v2',
    profileUrl: 'https://api.linkedin.com/v2/userinfo',
    profileMapping: { id: 'sub', username: 'name', avatar: 'picture', name: 'name' },
  },

  facebook: {
    id: 'facebook',
    name: 'Facebook',
    authorizeUrl: 'https://www.facebook.com/v21.0/dialog/oauth',
    tokenUrl: 'https://graph.facebook.com/v21.0/oauth/access_token',
    scopes: ['pages_manage_posts', 'pages_read_engagement', 'pages_show_list', 'pages_read_user_content', 'public_profile'],
    clientIdEnv: 'FACEBOOK_CLIENT_ID',
    clientSecretEnv: 'FACEBOOK_CLIENT_SECRET',
    usePKCE: false,
    apiBaseUrl: 'https://graph.facebook.com/v21.0',
    profileUrl: 'https://graph.facebook.com/v21.0/me?fields=id,name,picture.type(large)',
    profileMapping: { id: 'id', username: 'name', avatar: 'picture.data.url', name: 'name' },
  },

  instagram: {
    id: 'instagram',
    name: 'Instagram',
    authorizeUrl: 'https://www.facebook.com/v21.0/dialog/oauth',
    tokenUrl: 'https://graph.facebook.com/v21.0/oauth/access_token',
    scopes: ['instagram_basic', 'instagram_content_publish', 'instagram_manage_insights', 'pages_show_list', 'pages_read_engagement'],
    clientIdEnv: 'FACEBOOK_CLIENT_ID',
    clientSecretEnv: 'FACEBOOK_CLIENT_SECRET',
    usePKCE: false,
    apiBaseUrl: 'https://graph.facebook.com/v21.0',
    profileUrl: 'https://graph.facebook.com/v21.0/me?fields=id,name,picture.type(large)',
    profileMapping: { id: 'id', username: 'name', avatar: 'picture.data.url', name: 'name' },
    extraAuthorizeParams: { 'config_id': '' }, // Meta requires config_id for IG
  },

  youtube: {
    id: 'youtube',
    name: 'YouTube',
    authorizeUrl: 'https://accounts.google.com/o/oauth2/v2/auth',
    tokenUrl: 'https://oauth2.googleapis.com/token',
    scopes: [
      'https://www.googleapis.com/auth/youtube.upload',
      'https://www.googleapis.com/auth/youtube.readonly',
      'https://www.googleapis.com/auth/yt-analytics.readonly',
      'https://www.googleapis.com/auth/youtube.force-ssl',
      'openid', 'profile', 'email',
    ],
    clientIdEnv: 'GOOGLE_CLIENT_ID',
    clientSecretEnv: 'GOOGLE_CLIENT_SECRET',
    usePKCE: false,
    apiBaseUrl: 'https://www.googleapis.com/youtube/v3',
    profileUrl: 'https://www.googleapis.com/oauth2/v2/userinfo',
    profileMapping: { id: 'id', username: 'name', avatar: 'picture', name: 'name' },
    extraAuthorizeParams: { access_type: 'offline', prompt: 'consent' },
  },

  tiktok: {
    id: 'tiktok',
    name: 'TikTok',
    authorizeUrl: 'https://www.tiktok.com/v2/auth/authorize/',
    tokenUrl: 'https://open.tiktokapis.com/v2/oauth/token/',
    scopes: ['user.info.basic', 'user.info.profile', 'user.info.stats', 'video.publish', 'video.list'],
    clientIdEnv: 'TIKTOK_CLIENT_KEY',
    clientSecretEnv: 'TIKTOK_CLIENT_SECRET',
    usePKCE: true,
    apiBaseUrl: 'https://open.tiktokapis.com/v2',
    profileUrl: 'https://open.tiktokapis.com/v2/user/info/?fields=open_id,display_name,avatar_url,follower_count',
    profileMapping: { id: 'data.user.open_id', username: 'data.user.display_name', avatar: 'data.user.avatar_url' },
    extraAuthorizeParams: { 'code_challenge_method': 'S256' },
    extraTokenParams: { 'grant_type': 'authorization_code' },
  },

  pinterest: {
    id: 'pinterest',
    name: 'Pinterest',
    authorizeUrl: 'https://www.pinterest.com/oauth/',
    tokenUrl: 'https://api.pinterest.com/v5/oauth/token',
    scopes: ['boards:read', 'boards:write', 'pins:read', 'pins:write', 'user_accounts:read'],
    clientIdEnv: 'PINTEREST_CLIENT_ID',
    clientSecretEnv: 'PINTEREST_CLIENT_SECRET',
    usePKCE: false,
    apiBaseUrl: 'https://api.pinterest.com/v5',
    profileUrl: 'https://api.pinterest.com/v5/user_account',
    profileMapping: { id: 'username', username: 'username', avatar: 'profile_image' },
    tokenAuthMethod: 'basic',
  },

  reddit: {
    id: 'reddit',
    name: 'Reddit',
    authorizeUrl: 'https://www.reddit.com/api/v1/authorize',
    tokenUrl: 'https://www.reddit.com/api/v1/access_token',
    scopes: ['identity', 'read', 'submit', 'edit', 'history', 'mysubreddits'],
    clientIdEnv: 'REDDIT_CLIENT_ID',
    clientSecretEnv: 'REDDIT_CLIENT_SECRET',
    usePKCE: false,
    apiBaseUrl: 'https://oauth.reddit.com',
    profileUrl: 'https://oauth.reddit.com/api/v1/me',
    profileMapping: { id: 'id', username: 'name', avatar: 'icon_img' },
    tokenAuthMethod: 'basic',
    extraAuthorizeParams: { duration: 'permanent' },
  },

  medium: {
    id: 'medium',
    name: 'Medium',
    authorizeUrl: 'https://medium.com/m/oauth/authorize',
    tokenUrl: 'https://api.medium.com/v1/tokens',
    scopes: ['basicProfile', 'publishPost', 'listPublications'],
    clientIdEnv: 'MEDIUM_CLIENT_ID',
    clientSecretEnv: 'MEDIUM_CLIENT_SECRET',
    usePKCE: false,
    apiBaseUrl: 'https://api.medium.com/v1',
    profileUrl: 'https://api.medium.com/v1/me',
    profileMapping: { id: 'data.id', username: 'data.username', avatar: 'data.imageUrl', name: 'data.name' },
  },

  discord: {
    id: 'discord',
    name: 'Discord',
    authorizeUrl: 'https://discord.com/oauth2/authorize',
    tokenUrl: 'https://discord.com/api/oauth2/token',
    scopes: ['identify', 'guilds', 'bot', 'messages.read'],
    clientIdEnv: 'DISCORD_CLIENT_ID',
    clientSecretEnv: 'DISCORD_CLIENT_SECRET',
    usePKCE: false,
    apiBaseUrl: 'https://discord.com/api/v10',
    profileUrl: 'https://discord.com/api/v10/users/@me',
    profileMapping: { id: 'id', username: 'username', avatar: 'avatar' },
    tokenResponseType: 'form',
  },

  tumblr: {
    id: 'tumblr',
    name: 'Tumblr',
    authorizeUrl: 'https://www.tumblr.com/oauth2/authorize',
    tokenUrl: 'https://api.tumblr.com/v2/oauth2/token',
    scopes: ['basic', 'write', 'offline_access'],
    clientIdEnv: 'TUMBLR_CLIENT_ID',
    clientSecretEnv: 'TUMBLR_CLIENT_SECRET',
    usePKCE: false,
    apiBaseUrl: 'https://api.tumblr.com/v2',
    profileUrl: 'https://api.tumblr.com/v2/user/info',
    profileMapping: { id: 'response.user.name', username: 'response.user.name' },
  },
};

/** Get a platform config by ID. Throws if unsupported. */
export function getPlatformConfig(platformId: string): OAuthPlatformConfig {
  const config = OAUTH_PLATFORMS[platformId];
  if (!config) {
    throw new Error(`Unsupported OAuth platform: ${platformId}`);
  }
  return config;
}

/**
 * Get client credentials for a platform.
 * Priority: DB (oauth_credentials table) → env vars (.env.local).
 * DB credentials allow users to configure keys from the UI without editing files.
 */
export async function getClientCredentials(config: OAuthPlatformConfig): Promise<{ clientId: string; clientSecret: string }> {
  // 1. Try DB first
  try {
    const { db } = await import('@/lib/db');
    const { oauthCredentials } = await import('@/lib/db/schema');
    const { eq } = await import('drizzle-orm');
    const { decryptToken } = await import('./crypto');

    const rows: Array<{ clientId: string; clientSecret: string; status: string | null }> = await db
      .select({
        clientId: oauthCredentials.clientId,
        clientSecret: oauthCredentials.clientSecret,
        status: oauthCredentials.status,
      })
      .from(oauthCredentials)
      .where(eq(oauthCredentials.platform, config.id))
      .limit(1);

    if (rows.length > 0 && rows[0].status === 'active') {
      return {
        clientId: rows[0].clientId,
        clientSecret: decryptToken(rows[0].clientSecret),
      };
    }
  } catch {
    // DB not available — fall through to env vars
  }

  // 2. Fall back to env vars
  const clientId = process.env[config.clientIdEnv];
  const clientSecret = process.env[config.clientSecretEnv];
  if (!clientId || !clientSecret) {
    throw new Error(
      `Missing OAuth credentials for ${config.name}.\n` +
      `  Configure them in the Connect dialog, or set ${config.clientIdEnv} and ${config.clientSecretEnv} in .env.local`
    );
  }
  return { clientId, clientSecret };
}

/** Get the OAuth callback URL for a platform. */
export function getCallbackUrl(platformId: string): string {
  const base = process.env.NEXT_PUBLIC_APP_URL || 'http://localhost:3000';
  return `${base}/api/oauth/${platformId}/callback`;
}

/** Resolve a dot-path on an object (e.g., "data.user.name" → obj.data.user.name). */
export function resolvePath(obj: Record<string, unknown>, path: string): string | undefined {
  const parts = path.split('.');
  let current: unknown = obj;
  for (const part of parts) {
    if (current == null || typeof current !== 'object') return undefined;
    current = (current as Record<string, unknown>)[part];
  }
  return typeof current === 'string' ? current : current != null ? String(current) : undefined;
}
