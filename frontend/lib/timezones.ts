/**
 * Static list of globally-referenced timezones.
 * Used instead of Intl.supportedValuesOf('timeZone') to avoid SSR hydration mismatches.
 */
export const TIMEZONES = [
  // UTC
  'UTC',
  // Americas
  'America/New_York',
  'America/Chicago',
  'America/Denver',
  'America/Los_Angeles',
  'America/Anchorage',
  'America/Phoenix',
  'America/Toronto',
  'America/Vancouver',
  'America/Mexico_City',
  'America/Bogota',
  'America/Lima',
  'America/Santiago',
  'America/Sao_Paulo',
  'America/Buenos_Aires',
  // Europe
  'Europe/London',
  'Europe/Dublin',
  'Europe/Paris',
  'Europe/Berlin',
  'Europe/Madrid',
  'Europe/Rome',
  'Europe/Amsterdam',
  'Europe/Brussels',
  'Europe/Zurich',
  'Europe/Vienna',
  'Europe/Stockholm',
  'Europe/Oslo',
  'Europe/Copenhagen',
  'Europe/Helsinki',
  'Europe/Warsaw',
  'Europe/Prague',
  'Europe/Bucharest',
  'Europe/Athens',
  'Europe/Istanbul',
  'Europe/Moscow',
  // Africa
  'Africa/Cairo',
  'Africa/Lagos',
  'Africa/Johannesburg',
  'Africa/Nairobi',
  'Africa/Casablanca',
  // Middle East
  'Asia/Dubai',
  'Asia/Riyadh',
  'Asia/Tehran',
  'Asia/Jerusalem',
  // South Asia
  'Asia/Karachi',
  'Asia/Kolkata',
  'Asia/Colombo',
  'Asia/Dhaka',
  // East / Southeast Asia
  'Asia/Bangkok',
  'Asia/Jakarta',
  'Asia/Singapore',
  'Asia/Kuala_Lumpur',
  'Asia/Ho_Chi_Minh',
  'Asia/Manila',
  'Asia/Shanghai',
  'Asia/Hong_Kong',
  'Asia/Taipei',
  'Asia/Seoul',
  'Asia/Tokyo',
  // Oceania
  'Australia/Sydney',
  'Australia/Melbourne',
  'Australia/Brisbane',
  'Australia/Perth',
  'Australia/Adelaide',
  'Pacific/Auckland',
  'Pacific/Fiji',
  'Pacific/Honolulu',
] as const;

export type Timezone = typeof TIMEZONES[number];

/** Format a timezone ID for display: "Asia/Kolkata" → "Asia / Kolkata" */
export function formatTzLabel(tz: string): string {
  return tz.replace(/_/g, ' ').replace(/\//g, ' / ');
}
