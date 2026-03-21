import { describe, it, expect } from 'vitest';
import { TIMEZONES, formatTzLabel } from '../../lib/timezones';

describe('Timezone Formatting', () => {
  it('formats timestamp in UTC', () => {
    const iso = '2026-03-21T15:30:45.123Z';
    const d = new Date(iso);
    const formatted = d.toLocaleTimeString('en-US', {
      hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit',
      timeZone: 'UTC',
    });
    expect(formatted).toBe('15:30:45');
  });

  it('formats timestamp in different timezone', () => {
    const iso = '2026-03-21T15:30:45.123Z';
    const d = new Date(iso);
    const eastern = d.toLocaleTimeString('en-US', {
      hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit',
      timeZone: 'America/New_York',
    });
    // Eastern is UTC-4 in March (EDT)
    expect(eastern).toBe('11:30:45');
  });

  it('static timezone list contains major zones', () => {
    expect(TIMEZONES.length).toBeGreaterThan(50);
    expect(TIMEZONES).toContain('UTC');
    expect(TIMEZONES).toContain('America/New_York');
    expect(TIMEZONES).toContain('Asia/Tokyo');
    expect(TIMEZONES).toContain('Europe/London');
    expect(TIMEZONES).toContain('Asia/Kolkata');
    expect(TIMEZONES).toContain('Australia/Sydney');
  });

  it('formatTzLabel replaces underscores and slashes', () => {
    expect(formatTzLabel('America/New_York')).toBe('America / New York');
    expect(formatTzLabel('Asia/Ho_Chi_Minh')).toBe('Asia / Ho Chi Minh');
    expect(formatTzLabel('UTC')).toBe('UTC');
  });

  it('browser default timezone is valid', () => {
    const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
    expect(typeof tz).toBe('string');
    expect(tz.length).toBeGreaterThan(0);
  });
});
