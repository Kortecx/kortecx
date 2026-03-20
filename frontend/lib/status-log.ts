import { db, logs } from '@/lib/db';

/**
 * Write a status log entry to NeonDB.
 * Fire-and-forget — never blocks the caller.
 */
export function logStatus(
  level: 'info' | 'warning' | 'error',
  message: string,
  source: string,
  metadata?: Record<string, unknown>,
) {
  db.insert(logs).values({
    level,
    message,
    source,
    metadata: metadata ?? null,
  }).catch((err: unknown) => console.error('[status-log]', err));
}
