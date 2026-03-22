/**
 * DB Connection Factory
 *
 * Cloud (production / Neon cloud):
 *   DATABASE_URL = postgres://user:pass@ep-xxx.neon.tech/neondb?sslmode=require
 *   → uses @neondatabase/serverless (HTTP transport, edge-compatible)
 *
 * Local NeonDB proxy (neon dev --db local / docker neon-local):
 *   DATABASE_URL = postgresql://user:pass@localhost:5432/neondb
 *   DB_MODE = local
 *   → uses drizzle-orm/node-postgres (standard pg driver)
 *
 * Both modes use the exact same Drizzle schema.
 */

import * as schema from './schema';

function createDb() {
  const url = process.env.DATABASE_URL;
  const mode = process.env.DB_MODE; // 'local' | undefined

  if (!url) {
    throw new Error(
      'DATABASE_URL is not set.\n' +
      '  Cloud: Add your Neon connection string to .env.local\n' +
      '  Local: Run `docker compose up db` then set DATABASE_URL=postgresql://kortecx:kortecx@localhost:5433/kortecx_dev\n' +
      '  See: https://neon.tech/docs/get-started-with-neon/connect-neon'
    );
  }

  // Local mode — use standard node-postgres driver
  if (mode === 'local' || url.includes('localhost') || url.includes('127.0.0.1')) {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { drizzle } = require('drizzle-orm/node-postgres');
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { Pool }    = require('pg');
    const pool = new Pool({ connectionString: url });
    return drizzle(pool, { schema });
  }

  // Cloud / Neon serverless mode (default)
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { neon }    = require('@neondatabase/serverless');
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { drizzle } = require('drizzle-orm/neon-http');
  const sql = neon(url);
  return drizzle(sql, { schema });
}

// Singleton — reuse across hot-reloads in dev
const globalForDb = globalThis as unknown as {
  _db: ReturnType<typeof createDb> | undefined;
};

export const db = globalForDb._db ?? createDb();

if (process.env.NODE_ENV !== 'production') {
  globalForDb._db = db;
}

export * from './schema';
