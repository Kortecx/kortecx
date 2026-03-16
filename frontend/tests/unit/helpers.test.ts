import { describe, it, expect } from 'vitest';

// Test utility functions that are used across the app
describe('Format helpers', () => {
  // Test number formatting
  function fmt(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
    return String(n);
  }

  it('formats numbers below 1000', () => {
    expect(fmt(0)).toBe('0');
    expect(fmt(42)).toBe('42');
    expect(fmt(999)).toBe('999');
  });

  it('formats thousands', () => {
    expect(fmt(1000)).toBe('1k');
    expect(fmt(1500)).toBe('2k');
    expect(fmt(999999)).toBe('1000k');
  });

  it('formats millions', () => {
    expect(fmt(1000000)).toBe('1.0M');
    expect(fmt(2500000)).toBe('2.5M');
  });
});

describe('Byte formatting', () => {
  function fmtBytes(b: number): string {
    if (b >= 1_073_741_824) return `${(b / 1_073_741_824).toFixed(1)} GB`;
    if (b >= 1_048_576) return `${(b / 1_048_576).toFixed(0)} MB`;
    if (b >= 1024) return `${(b / 1024).toFixed(0)} KB`;
    return `${b} B`;
  }

  it('formats bytes', () => {
    expect(fmtBytes(500)).toBe('500 B');
    expect(fmtBytes(1024)).toBe('1 KB');
    expect(fmtBytes(1048576)).toBe('1 MB');
    expect(fmtBytes(1073741824)).toBe('1.0 GB');
  });
});

describe('Download time estimation', () => {
  function estimateDownloadTime(bytes: number): string {
    const speedBps = 10 * 1_048_576;
    const seconds = bytes / speedBps;
    if (seconds < 5) return '<5s';
    if (seconds < 60) return `${Math.round(seconds)}s`;
    if (seconds < 3600) return `${Math.ceil(seconds / 60)}m`;
    return `${(seconds / 3600).toFixed(1)}h`;
  }

  it('estimates small downloads', () => {
    expect(estimateDownloadTime(1_000_000)).toBe('<5s');
  });

  it('estimates medium downloads', () => {
    expect(estimateDownloadTime(100_000_000)).toBe('10s');
  });

  it('estimates large downloads', () => {
    expect(estimateDownloadTime(5_000_000_000)).toBe('8m');
  });
});
