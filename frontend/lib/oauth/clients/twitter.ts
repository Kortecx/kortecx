/**
 * Twitter/X API Client — consume analytics, create tweets/threads, manage lists.
 * Uses Twitter API v2 with OAuth 2.0 user context (not app-only).
 */

import { BasePlatformClient, type PlatformApiResponse } from './base';

export interface Tweet {
  id: string;
  text: string;
  created_at?: string;
  public_metrics?: {
    retweet_count: number;
    reply_count: number;
    like_count: number;
    quote_count: number;
    impression_count: number;
    bookmark_count: number;
  };
}

export interface TwitterUser {
  id: string;
  username: string;
  name: string;
  profile_image_url?: string;
  public_metrics?: {
    followers_count: number;
    following_count: number;
    tweet_count: number;
    listed_count: number;
  };
}

export class TwitterClient extends BasePlatformClient {
  readonly platform = 'twitter';
  readonly apiBase = 'https://api.twitter.com/2';

  /** Consume: Get authenticated user's profile and metrics. */
  async getMe(): Promise<PlatformApiResponse<TwitterUser>> {
    const res = await this.apiFetch<{ data: TwitterUser }>(
      '/users/me?user.fields=profile_image_url,public_metrics,description,created_at',
      { operation: 'consume' },
    );
    return res.success ? { success: true, data: res.data?.data } : { success: false, error: res.error, permissionDenied: res.permissionDenied };
  }

  /** Consume: Get user's recent tweets with metrics. */
  async getMyTweets(maxResults = 10): Promise<PlatformApiResponse<Tweet[]>> {
    const res = await this.apiFetch<{ data: Tweet[] }>(
      `/users/me/tweets?max_results=${maxResults}&tweet.fields=created_at,public_metrics`,
      { operation: 'consume' },
    );
    return res.success ? { success: true, data: res.data?.data || [] } : { success: false, error: res.error, permissionDenied: res.permissionDenied };
  }

  /** Report: Get tweet metrics by ID. */
  async getTweetMetrics(tweetId: string): Promise<PlatformApiResponse<Tweet>> {
    const res = await this.apiFetch<{ data: Tweet }>(
      `/tweets/${tweetId}?tweet.fields=public_metrics,created_at`,
      { operation: 'report' },
    );
    return res.success ? { success: true, data: res.data?.data } : { success: false, error: res.error, permissionDenied: res.permissionDenied };
  }

  /** Publish: Create a new tweet. */
  async createTweet(text: string): Promise<PlatformApiResponse<{ id: string; text: string }>> {
    const res = await this.apiFetch<{ data: { id: string; text: string } }>('/tweets', {
      method: 'POST',
      body: JSON.stringify({ text }),
      operation: 'publish',
    });
    return res.success ? { success: true, data: res.data?.data } : { success: false, error: res.error, permissionDenied: res.permissionDenied };
  }

  /** Publish: Create a thread (multiple tweets linked via reply_to). */
  async createThread(tweets: string[]): Promise<PlatformApiResponse<string[]>> {
    const denied = await this.checkPermission<string[]>('publish');
    if (denied) return denied;

    const ids: string[] = [];
    let replyTo: string | undefined;

    for (const text of tweets) {
      const body: Record<string, unknown> = { text };
      if (replyTo) {
        body.reply = { in_reply_to_tweet_id: replyTo };
      }

      const res = await this.apiFetch<{ data: { id: string } }>('/tweets', {
        method: 'POST',
        body: JSON.stringify(body),
      });

      if (!res.success) return { success: false, error: res.error };
      const id = res.data?.data.id;
      if (id) {
        ids.push(id);
        replyTo = id;
      }
    }

    return { success: true, data: ids };
  }

  /** Execute: Delete a tweet. */
  async deleteTweet(tweetId: string): Promise<PlatformApiResponse<boolean>> {
    const res = await this.apiFetch<{ data: { deleted: boolean } }>(`/tweets/${tweetId}`, {
      method: 'DELETE',
      operation: 'execute',
    });
    return res.success ? { success: true, data: res.data?.data.deleted } : { success: false, error: res.error, permissionDenied: res.permissionDenied };
  }

  /** Execute: Like a tweet. */
  async likeTweet(userId: string, tweetId: string): Promise<PlatformApiResponse<boolean>> {
    const res = await this.apiFetch<{ data: { liked: boolean } }>(`/users/${userId}/likes`, {
      method: 'POST',
      body: JSON.stringify({ tweet_id: tweetId }),
      operation: 'execute',
    });
    return res.success ? { success: true, data: res.data?.data.liked } : { success: false, error: res.error, permissionDenied: res.permissionDenied };
  }

  /** Consume: Search recent tweets. */
  async searchTweets(query: string, maxResults = 10): Promise<PlatformApiResponse<Tweet[]>> {
    const res = await this.apiFetch<{ data: Tweet[] }>(
      `/tweets/search/recent?query=${encodeURIComponent(query)}&max_results=${maxResults}&tweet.fields=public_metrics,created_at`,
      { operation: 'consume' },
    );
    return res.success ? { success: true, data: res.data?.data || [] } : { success: false, error: res.error, permissionDenied: res.permissionDenied };
  }
}

export const twitterClient = new TwitterClient();
