import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  THEME_STORAGE_KEY,
  applyResolvedTheme,
  getResolvedTheme,
  getThemePreference,
  loadThemePreference,
  resetThemeStoreForTests,
  resolveTheme,
  saveThemePreference,
  setThemePreference,
  subscribeTheme,
} from "../../src/lib/theme";

beforeEach(() => {
  localStorage.clear();
  resetThemeStoreForTests();
  delete document.documentElement.dataset.theme;
});

afterEach(() => {
  localStorage.clear();
  resetThemeStoreForTests();
});

describe("resolveTheme", () => {
  it("passes explicit preferences through untouched", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });

  it("follows the OS signal for 'system'", () => {
    expect(resolveTheme("system", false)).toBe("light");
    expect(resolveTheme("system", true)).toBe("dark");
  });
});

describe("persistence", () => {
  it("defaults to 'system' when nothing is stored", () => {
    expect(loadThemePreference()).toBe("system");
  });

  it("round-trips a stored preference as a RAW string (the inline-script contract)", () => {
    saveThemePreference("dark");
    // raw, not JSON — index.html's pre-paint script compares with ===
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");
    expect(loadThemePreference()).toBe("dark");
  });

  it("degrades a foreign stored value to 'system' (corruption-safe)", () => {
    localStorage.setItem(THEME_STORAGE_KEY, '"dark"'); // a JSON-stringified relic
    expect(loadThemePreference()).toBe("system");
    localStorage.setItem(THEME_STORAGE_KEY, "neon");
    expect(loadThemePreference()).toBe("system");
  });
});

describe("the store", () => {
  it("setThemePreference persists, stamps <html data-theme>, and notifies", () => {
    const seen = vi.fn();
    const unsubscribe = subscribeTheme(seen);

    setThemePreference("dark");
    expect(getThemePreference()).toBe("dark");
    expect(getResolvedTheme()).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");
    expect(seen).toHaveBeenCalledTimes(1);

    unsubscribe();
    setThemePreference("light");
    expect(seen).toHaveBeenCalledTimes(1); // unsubscribed — no further calls
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("'system' resolves light when matchMedia is unavailable (jsdom)", () => {
    // jsdom ships no matchMedia; the store must degrade, never throw.
    setThemePreference("system");
    expect(getResolvedTheme()).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("applyResolvedTheme stamps the stored preference on a fresh load", () => {
    localStorage.setItem(THEME_STORAGE_KEY, "dark");
    resetThemeStoreForTests();
    applyResolvedTheme();
    expect(document.documentElement.dataset.theme).toBe("dark");
  });
});
