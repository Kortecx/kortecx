/**
 * Platform Client Registry — unified access to all social platform API clients.
 *
 * Usage:
 *   import { getPlatformClient } from '@/lib/oauth/clients';
 *   const client = getPlatformClient('twitter');
 *   const tweets = await client.getMyTweets();
 */

export { twitterClient, TwitterClient } from './twitter';
export { linkedinClient, LinkedInClient } from './linkedin';
export { facebookClient, FacebookClient } from './facebook';
export { youtubeClient, YouTubeClient } from './youtube';
export { BasePlatformClient } from './base';
export type { PlatformApiResponse } from './base';

import { twitterClient } from './twitter';
import { linkedinClient } from './linkedin';
import { facebookClient } from './facebook';
import { youtubeClient } from './youtube';
import type { BasePlatformClient } from './base';

const clientRegistry: Record<string, BasePlatformClient> = {
  twitter: twitterClient,
  linkedin: linkedinClient,
  facebook: facebookClient,
  youtube: youtubeClient,
};

/** Get a platform API client by platform ID. */
export function getPlatformClient(platform: string): BasePlatformClient {
  const client = clientRegistry[platform];
  if (!client) {
    throw new Error(
      `No API client available for platform: ${platform}. ` +
      `Supported: ${Object.keys(clientRegistry).join(', ')}`
    );
  }
  return client;
}

/** List all platforms that have API clients. */
export function getSupportedPlatforms(): string[] {
  return Object.keys(clientRegistry);
}
