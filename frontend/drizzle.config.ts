import type { Config } from 'drizzle-kit';

const url = process.env.DATABASE_URL!;
const isLocal = process.env.DB_MODE === 'local' ||
                url?.includes('localhost') ||
                url?.includes('127.0.0.1');

export default {
  schema:    './lib/db/schema.ts',
  out:       './drizzle',
  dialect:   'postgresql',
  dbCredentials: { url },
  // For local postgres — disable SSL
  ...(isLocal ? {} : {}),
} satisfies Config;
