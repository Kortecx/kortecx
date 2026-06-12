/**
 * Theme preference store (PR-0 dual theme) — pure logic + a tiny module store; no
 * React here (the `useTheme` hook in app/use-theme.ts subscribes via
 * `useSyncExternalStore`). Persistence mirrors the chat-settings pattern
 * (localStorage with try/catch fallbacks; corruption-safe).
 *
 * The stored value is the RAW string ("system" | "light" | "dark") — NOT JSON —
 * because the pre-paint inline script in index.html reads the same key with
 * `localStorage.getItem` only. Keep STORAGE_KEY + the resolve semantics in sync
 * with that script: dark ⇔ stored "dark", or anything-but-"light" while the OS
 * prefers dark.
 */

export type ThemePreference = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

export const THEME_STORAGE_KEY = "kortecx.ui.theme";

export const THEME_PREFERENCES: readonly ThemePreference[] = ["system", "light", "dark"];

function isThemePreference(v: unknown): v is ThemePreference {
  return v === "system" || v === "light" || v === "dark";
}

/** Pure: preference + the OS signal → the palette to render. */
export function resolveTheme(pref: ThemePreference, systemDark: boolean): ResolvedTheme {
  if (pref === "system") {
    return systemDark ? "dark" : "light";
  }
  return pref;
}

/** Load the stored preference; anything missing/foreign degrades to "system". */
export function loadThemePreference(): ThemePreference {
  try {
    const raw = localStorage.getItem(THEME_STORAGE_KEY);
    return isThemePreference(raw) ? raw : "system";
  } catch {
    return "system";
  }
}

/** Persist (best-effort; storage may be unavailable in private mode). */
export function saveThemePreference(pref: ThemePreference): void {
  try {
    localStorage.setItem(THEME_STORAGE_KEY, pref);
  } catch {
    /* best-effort */
  }
}

/** Does the OS prefer dark right now? jsdom has no matchMedia — degrade to false. */
export function systemPrefersDark(): boolean {
  try {
    return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
  } catch {
    return false;
  }
}

// ---------------------------------------------------------------------------
// Module store: one preference, one OS listener, N subscribers.
// ---------------------------------------------------------------------------

let preference: ThemePreference | null = null;
let osListenerAttached = false;
const listeners = new Set<() => void>();

function currentPreference(): ThemePreference {
  if (preference === null) {
    preference = loadThemePreference();
  }
  return preference;
}

function notify(): void {
  for (const cb of listeners) {
    cb();
  }
}

/** Attach the prefers-color-scheme listener once (first subscriber). */
function attachOsListener(): void {
  if (osListenerAttached) {
    return;
  }
  osListenerAttached = true;
  try {
    window.matchMedia?.("(prefers-color-scheme: dark)").addEventListener("change", () => {
      applyResolvedTheme();
      notify();
    });
  } catch {
    /* no matchMedia (jsdom) — "system" just stays light */
  }
}

export function getThemePreference(): ThemePreference {
  return currentPreference();
}

export function getResolvedTheme(): ResolvedTheme {
  return resolveTheme(currentPreference(), systemPrefersDark());
}

/** Stamp the resolved palette onto <html> (the selector app.css keys on). */
export function applyResolvedTheme(): void {
  document.documentElement.dataset.theme = getResolvedTheme();
}

/** Set + persist + re-stamp + notify subscribers. */
export function setThemePreference(pref: ThemePreference): void {
  preference = pref;
  saveThemePreference(pref);
  applyResolvedTheme();
  notify();
}

/** Subscribe to preference/OS changes (useSyncExternalStore-shaped). */
export function subscribeTheme(cb: () => void): () => void {
  attachOsListener();
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

/** Test-only: drop the cached preference so the next read hits localStorage. */
export function resetThemeStoreForTests(): void {
  preference = null;
}
