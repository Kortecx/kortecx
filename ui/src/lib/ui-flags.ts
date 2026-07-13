/**
 * Best-effort boolean UI flags persisted to localStorage — collapsed rails, docked
 * panels, and other layout toggles that should survive a reload. Never throws: storage
 * may be disabled (private mode) or absent (jsdom / SSR), in which case the flag reads
 * as `false` and writes are dropped silently.
 */

export function loadFlag(key: string): boolean {
  try {
    return localStorage.getItem(key) === "1";
  } catch {
    return false;
  }
}

export function persistFlag(key: string, value: boolean): void {
  try {
    localStorage.setItem(key, value ? "1" : "0");
  } catch {
    /* best-effort */
  }
}
