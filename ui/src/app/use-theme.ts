/**
 * React binding for the theme store (lib/theme.ts) — `useSyncExternalStore` over
 * the module store, so every consumer (Settings chips, Monaco) re-renders on a
 * preference change OR an OS prefers-color-scheme flip.
 */

import { useSyncExternalStore } from "react";
import {
  type ResolvedTheme,
  type ThemePreference,
  getResolvedTheme,
  getThemePreference,
  setThemePreference,
  subscribeTheme,
} from "../lib/theme";

export interface ThemeControls {
  /** The stored choice ("system" | "light" | "dark"). */
  readonly preference: ThemePreference;
  /** The palette actually rendering ("light" | "dark"). */
  readonly resolved: ResolvedTheme;
  readonly setPreference: (pref: ThemePreference) => void;
}

export function useTheme(): ThemeControls {
  const preference = useSyncExternalStore(
    subscribeTheme,
    getThemePreference,
    (): ThemePreference => "system",
  );
  const resolved = useSyncExternalStore(
    subscribeTheme,
    getResolvedTheme,
    (): ResolvedTheme => "light",
  );
  return { preference, resolved, setPreference: setThemePreference };
}
