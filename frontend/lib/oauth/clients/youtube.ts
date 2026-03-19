/**
 * YouTube Data API Client — manage channels, videos, playlists, and analytics.
 * Uses YouTube Data API v3 + YouTube Analytics API with Google OAuth 2.0.
 */

import { BasePlatformClient, type PlatformApiResponse } from './base';

export interface YouTubeChannel {
  id: string;
  title: string;
  description: string;
  customUrl?: string;
  thumbnailUrl?: string;
  subscriberCount: number;
  videoCount: number;
  viewCount: number;
}

export interface YouTubeVideo {
  id: string;
  title: string;
  description: string;
  publishedAt: string;
  thumbnailUrl?: string;
  viewCount: number;
  likeCount: number;
  commentCount: number;
  duration?: string;
}

export class YouTubeClient extends BasePlatformClient {
  readonly platform = 'youtube';
  readonly apiBase = 'https://www.googleapis.com/youtube/v3';

  /** Consume: Get authenticated user's channel info. */
  async getMyChannel(): Promise<PlatformApiResponse<YouTubeChannel>> {
    const res = await this.apiFetch<{ items: Array<{
      id: string;
      snippet: { title: string; description: string; customUrl?: string; thumbnails?: { default?: { url: string } } };
      statistics: { subscriberCount: string; videoCount: string; viewCount: string };
    }> }>(
      '/channels?part=snippet,statistics&mine=true',
    );

    if (!res.success || !res.data?.items?.[0]) return { success: false, error: 'Channel not found' };
    const ch = res.data.items[0];
    return {
      success: true,
      data: {
        id: ch.id,
        title: ch.snippet.title,
        description: ch.snippet.description,
        customUrl: ch.snippet.customUrl,
        thumbnailUrl: ch.snippet.thumbnails?.default?.url,
        subscriberCount: parseInt(ch.statistics.subscriberCount) || 0,
        videoCount: parseInt(ch.statistics.videoCount) || 0,
        viewCount: parseInt(ch.statistics.viewCount) || 0,
      },
    };
  }

  /** Consume: Get recent videos with metrics. */
  async getMyVideos(maxResults = 10): Promise<PlatformApiResponse<YouTubeVideo[]>> {
    // First get video IDs from channel uploads
    const channelRes = await this.apiFetch<{ items: Array<{ contentDetails: { relatedPlaylists: { uploads: string } } }> }>(
      '/channels?part=contentDetails&mine=true',
    );
    if (!channelRes.success || !channelRes.data?.items?.[0]) return { success: true, data: [] };

    const uploadsPlaylistId = channelRes.data.items[0].contentDetails.relatedPlaylists.uploads;
    const playlistRes = await this.apiFetch<{ items: Array<{ contentDetails: { videoId: string } }> }>(
      `/playlistItems?part=contentDetails&playlistId=${uploadsPlaylistId}&maxResults=${maxResults}`,
    );
    if (!playlistRes.success || !playlistRes.data?.items) return { success: true, data: [] };

    const videoIds = playlistRes.data.items.map(i => i.contentDetails.videoId).join(',');
    if (!videoIds) return { success: true, data: [] };

    const videosRes = await this.apiFetch<{ items: Array<{
      id: string;
      snippet: { title: string; description: string; publishedAt: string; thumbnails?: { medium?: { url: string } } };
      statistics: { viewCount: string; likeCount: string; commentCount: string };
      contentDetails?: { duration?: string };
    }> }>(
      `/videos?part=snippet,statistics,contentDetails&id=${videoIds}`,
    );

    if (!videosRes.success || !videosRes.data?.items) return { success: true, data: [] };

    return {
      success: true,
      data: videosRes.data.items.map(v => ({
        id: v.id,
        title: v.snippet.title,
        description: v.snippet.description,
        publishedAt: v.snippet.publishedAt,
        thumbnailUrl: v.snippet.thumbnails?.medium?.url,
        viewCount: parseInt(v.statistics.viewCount) || 0,
        likeCount: parseInt(v.statistics.likeCount) || 0,
        commentCount: parseInt(v.statistics.commentCount) || 0,
        duration: v.contentDetails?.duration,
      })),
    };
  }

  /** Consume: Get video analytics. */
  async getVideoAnalytics(videoId: string): Promise<PlatformApiResponse<YouTubeVideo>> {
    const res = await this.apiFetch<{ items: Array<{
      id: string;
      snippet: { title: string; description: string; publishedAt: string };
      statistics: { viewCount: string; likeCount: string; commentCount: string };
    }> }>(
      `/videos?part=snippet,statistics&id=${videoId}`,
    );

    if (!res.success || !res.data?.items?.[0]) return { success: false, error: 'Video not found' };
    const v = res.data.items[0];
    return {
      success: true,
      data: {
        id: v.id,
        title: v.snippet.title,
        description: v.snippet.description,
        publishedAt: v.snippet.publishedAt,
        viewCount: parseInt(v.statistics.viewCount) || 0,
        likeCount: parseInt(v.statistics.likeCount) || 0,
        commentCount: parseInt(v.statistics.commentCount) || 0,
      },
    };
  }

  /** Execute: Update video metadata (title, description, tags). */
  async updateVideo(
    videoId: string,
    updates: { title?: string; description?: string; tags?: string[] },
  ): Promise<PlatformApiResponse<{ id: string }>> {
    // Need to get current snippet first
    const current = await this.apiFetch<{ items: Array<{ snippet: Record<string, unknown> }> }>(
      `/videos?part=snippet&id=${videoId}`,
    );
    if (!current.success || !current.data?.items?.[0]) return { success: false, error: 'Video not found' };

    const snippet = { ...current.data.items[0].snippet, ...updates };
    return this.apiFetch<{ id: string }>('/videos?part=snippet', {
      method: 'PUT',
      body: JSON.stringify({ id: videoId, snippet }),
    });
  }

  /** Report: Get channel analytics from YouTube Analytics API. */
  async getChannelReport(
    startDate: string,
    endDate: string,
    metrics = 'views,estimatedMinutesWatched,averageViewDuration,subscribersGained,likes',
  ): Promise<PlatformApiResponse<{ rows: unknown[][]; columnHeaders: Array<{ name: string }> }>> {
    return this.apiFetch(
      `https://youtubeanalytics.googleapis.com/v2/reports?ids=channel==MINE&startDate=${startDate}&endDate=${endDate}&metrics=${metrics}&dimensions=day&sort=day`,
    );
  }
}

export const youtubeClient = new YouTubeClient();
