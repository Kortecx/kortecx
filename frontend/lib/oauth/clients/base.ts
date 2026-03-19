/**
 * Base Platform Client — shared utilities for all platform API clients.
 * Provides authenticated fetch, permission enforcement, and rate limit handling.
 */

import { getValidAccessToken } from '../token-refresh';
import { db } from '@/lib/db';
import { socialConnections } from '@/lib/db/schema';
import { eq } from 'drizzle-orm';

export type OperationType = 'consume' | 'generate' | 'publish' | 'schedule' | 'report' | 'execute';

export interface PlatformApiResponse<T = unknown> {
  success: boolean;
  data?: T;
  error?: string;
  rateLimited?: boolean;
  permissionDenied?: boolean;
}

export abstract class BasePlatformClient {
  abstract readonly platform: string;
  abstract readonly apiBase: string;

  /**
   * Check if a specific operation is permitted for this platform connection.
   * Returns false if the user has explicitly disabled it in configure.
   */
  async isOperationPermitted(operation: OperationType): Promise<boolean> {
    try {
      const rows: Array<{ permissions: unknown }> = await db
        .select({ permissions: socialConnections.permissions })
        .from(socialConnections)
        .where(eq(socialConnections.platform, this.platform))
        .limit(1);

      if (rows.length === 0) return false;
      const perms = rows[0].permissions as Record<string, boolean> | null;
      if (!perms) return true; // no permissions set = all allowed
      return perms[operation] !== false;
    } catch {
      return true; // if DB fails, allow by default
    }
  }

  /**
   * Guard an API call by checking the operation permission first.
   * Returns a permissionDenied response if blocked.
   */
  protected async checkPermission<T>(operation: OperationType): Promise<PlatformApiResponse<T> | null> {
    const allowed = await this.isOperationPermitted(operation);
    if (!allowed) {
      return {
        success: false,
        error: `Operation "${operation}" is disabled for ${this.platform}. Enable it in Configure > Permissions.`,
        permissionDenied: true,
      };
    }
    return null;
  }

  /** Authenticated fetch against the platform API. */
  protected async apiFetch<T>(
    path: string,
    options?: RequestInit & { parseJson?: boolean; operation?: OperationType },
  ): Promise<PlatformApiResponse<T>> {
    try {
      // Check permission if operation specified
      if (options?.operation) {
        const denied = await this.checkPermission<T>(options.operation);
        if (denied) return denied;
      }

      const { accessToken, connectionId } = await getValidAccessToken(this.platform);

      const url = path.startsWith('http') ? path : `${this.apiBase}${path}`;
      const res = await fetch(url, {
        ...options,
        headers: {
          'Authorization': `Bearer ${accessToken}`,
          'Content-Type': 'application/json',
          'Accept': 'application/json',
          ...options?.headers,
        },
      });

      // Track usage
      await db.update(socialConnections)
        .set({ lastUsedAt: new Date() })
        .where(eq(socialConnections.id, connectionId));

      if (res.status === 429) {
        const retryAfter = res.headers.get('retry-after');
        return { success: false, error: `Rate limited. Retry after ${retryAfter || '?'} seconds.`, rateLimited: true };
      }

      if (!res.ok) {
        const errText = await res.text();
        return { success: false, error: `${this.platform} API error ${res.status}: ${errText}` };
      }

      const parseJson = options?.parseJson !== false;
      const data = parseJson ? (await res.json()) as T : undefined;
      return { success: true, data };
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unknown error';
      return { success: false, error: message };
    }
  }
}
