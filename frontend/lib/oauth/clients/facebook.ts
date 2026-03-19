/**
 * Facebook Graph API Client — manage pages, publish posts, consume insights.
 * Uses Facebook Graph API v21.0 with OAuth 2.0.
 */

import { BasePlatformClient, type PlatformApiResponse } from './base';

export interface FacebookPage {
  id: string;
  name: string;
  access_token: string;
  category: string;
  fan_count?: number;
}

export interface FacebookPost {
  id: string;
  message?: string;
  created_time: string;
  insights?: {
    impressions: number;
    reach: number;
    engagements: number;
    reactions: number;
    shares: number;
    comments: number;
  };
}

export class FacebookClient extends BasePlatformClient {
  readonly platform = 'facebook';
  readonly apiBase = 'https://graph.facebook.com/v21.0';

  /** Consume: Get user's managed pages. */
  async getPages(): Promise<PlatformApiResponse<FacebookPage[]>> {
    const res = await this.apiFetch<{ data: FacebookPage[] }>(
      '/me/accounts?fields=id,name,access_token,category,fan_count',
    );
    return res.success ? { success: true, data: res.data?.data || [] } : { success: false, error: res.error };
  }

  /** Publish: Create a post on a Facebook Page. */
  async createPagePost(pageId: string, pageToken: string, message: string, link?: string): Promise<PlatformApiResponse<{ id: string }>> {
    const body: Record<string, string> = { message };
    if (link) body.link = link;

    return this.apiFetch<{ id: string }>(`/${pageId}/feed`, {
      method: 'POST',
      headers: { 'Authorization': `Bearer ${pageToken}` },
      body: JSON.stringify(body),
    });
  }

  /** Publish: Schedule a post on a Facebook Page. */
  async schedulePagePost(
    pageId: string,
    pageToken: string,
    message: string,
    scheduledTime: Date,
  ): Promise<PlatformApiResponse<{ id: string }>> {
    const unixTime = Math.floor(scheduledTime.getTime() / 1000);
    return this.apiFetch<{ id: string }>(`/${pageId}/feed`, {
      method: 'POST',
      headers: { 'Authorization': `Bearer ${pageToken}` },
      body: JSON.stringify({
        message,
        published: false,
        scheduled_publish_time: unixTime,
      }),
    });
  }

  /** Consume: Get page post insights. */
  async getPostInsights(postId: string, pageToken: string): Promise<PlatformApiResponse<FacebookPost['insights']>> {
    const metrics = 'post_impressions,post_engaged_users,post_reactions_like_total,post_clicks';
    const res = await this.apiFetch<{ data: Array<{ name: string; values: Array<{ value: number }> }> }>(
      `/${postId}/insights?metric=${metrics}`,
      { headers: { 'Authorization': `Bearer ${pageToken}` } },
    );

    if (!res.success || !res.data?.data) return { success: true, data: undefined };

    const metricsMap: Record<string, number> = {};
    for (const m of res.data.data) {
      metricsMap[m.name] = m.values?.[0]?.value || 0;
    }

    return {
      success: true,
      data: {
        impressions: metricsMap['post_impressions'] || 0,
        reach: metricsMap['post_impressions_unique'] || 0,
        engagements: metricsMap['post_engaged_users'] || 0,
        reactions: metricsMap['post_reactions_like_total'] || 0,
        shares: 0,
        comments: 0,
      },
    };
  }

  /** Consume: Get page-level insights. */
  async getPageInsights(
    pageId: string,
    pageToken: string,
    period: 'day' | 'week' | 'days_28' = 'week',
  ): Promise<PlatformApiResponse<Record<string, number>>> {
    const metrics = 'page_impressions,page_engaged_users,page_fans,page_views_total';
    const res = await this.apiFetch<{ data: Array<{ name: string; values: Array<{ value: number }> }> }>(
      `/${pageId}/insights?metric=${metrics}&period=${period}`,
      { headers: { 'Authorization': `Bearer ${pageToken}` } },
    );

    if (!res.success || !res.data?.data) return { success: true, data: {} };

    const result: Record<string, number> = {};
    for (const m of res.data.data) {
      result[m.name] = m.values?.[0]?.value || 0;
    }
    return { success: true, data: result };
  }

  /** Execute: Delete a post. */
  async deletePost(postId: string, pageToken: string): Promise<PlatformApiResponse<boolean>> {
    const res = await this.apiFetch<{ success: boolean }>(`/${postId}`, {
      method: 'DELETE',
      headers: { 'Authorization': `Bearer ${pageToken}` },
    });
    return { success: res.success, data: res.success };
  }
}

export const facebookClient = new FacebookClient();
