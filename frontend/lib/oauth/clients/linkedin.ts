/**
 * LinkedIn API Client — publish articles/posts, consume analytics, manage shares.
 * Uses LinkedIn Marketing & Community Management APIs with OAuth 2.0.
 */

import { BasePlatformClient, type PlatformApiResponse } from './base';

export interface LinkedInProfile {
  sub: string;
  name: string;
  email?: string;
  picture?: string;
}

export interface LinkedInPost {
  id: string;
  author: string;
  commentary: string;
  visibility: string;
  createdAt?: number;
  stats?: {
    impressionCount: number;
    likeCount: number;
    commentCount: number;
    shareCount: number;
    clickCount: number;
    engagementRate: number;
  };
}

export class LinkedInClient extends BasePlatformClient {
  readonly platform = 'linkedin';
  readonly apiBase = 'https://api.linkedin.com/v2';

  /** Consume: Get authenticated user's profile. */
  async getMe(): Promise<PlatformApiResponse<LinkedInProfile>> {
    return this.apiFetch<LinkedInProfile>('https://api.linkedin.com/v2/userinfo');
  }

  /** Publish: Create a text post on LinkedIn. */
  async createPost(
    text: string,
    personUrn: string,
    visibility: 'PUBLIC' | 'CONNECTIONS' = 'PUBLIC',
  ): Promise<PlatformApiResponse<{ id: string }>> {
    const body = {
      author: personUrn,
      commentary: text,
      visibility,
      distribution: {
        feedDistribution: 'MAIN_FEED',
        targetEntities: [],
        thirdPartyDistributionChannels: [],
      },
      lifecycleState: 'PUBLISHED',
    };

    return this.apiFetch<{ id: string }>('/posts', {
      method: 'POST',
      headers: {
        'LinkedIn-Version': '202401',
        'X-Restli-Protocol-Version': '2.0.0',
      },
      body: JSON.stringify(body),
    });
  }

  /** Publish: Create an article post with a link. */
  async createArticlePost(
    text: string,
    articleUrl: string,
    title: string,
    personUrn: string,
  ): Promise<PlatformApiResponse<{ id: string }>> {
    const body = {
      author: personUrn,
      commentary: text,
      visibility: 'PUBLIC',
      distribution: {
        feedDistribution: 'MAIN_FEED',
        targetEntities: [],
        thirdPartyDistributionChannels: [],
      },
      content: {
        article: {
          source: articleUrl,
          title,
        },
      },
      lifecycleState: 'PUBLISHED',
    };

    return this.apiFetch<{ id: string }>('/posts', {
      method: 'POST',
      headers: {
        'LinkedIn-Version': '202401',
        'X-Restli-Protocol-Version': '2.0.0',
      },
      body: JSON.stringify(body),
    });
  }

  /** Consume: Get post analytics (requires Marketing API access). */
  async getPostStats(postUrn: string): Promise<PlatformApiResponse<LinkedInPost['stats']>> {
    const res = await this.apiFetch<{ elements: Array<{ totalShareStatistics: LinkedInPost['stats'] }> }>(
      `/organizationalEntityShareStatistics?q=organizationalEntity&shares=List(${encodeURIComponent(postUrn)})`,
      { headers: { 'LinkedIn-Version': '202401' } },
    );
    if (res.success && res.data?.elements?.[0]) {
      return { success: true, data: res.data.elements[0].totalShareStatistics };
    }
    return { success: true, data: undefined };
  }

  /** Execute: Delete a post. */
  async deletePost(postUrn: string): Promise<PlatformApiResponse<boolean>> {
    const res = await this.apiFetch(`/posts/${encodeURIComponent(postUrn)}`, {
      method: 'DELETE',
      headers: { 'LinkedIn-Version': '202401' },
    });
    return { success: res.success, data: res.success };
  }
}

export const linkedinClient = new LinkedInClient();
