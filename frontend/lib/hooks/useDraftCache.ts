'use client';

import { useEffect, useCallback, useRef } from 'react';

const CACHE_PREFIX = 'kortecx_draft_';

interface DraftEntry<T> {
  id: string;
  type: string;
  label: string;
  data: T;
  savedAt: string;
}

/**
 * Auto-save draft cache using localStorage.
 * Saves every `interval` ms and on beforeunload.
 */
export function useDraftCache<T>(type: string, id: string, options?: { interval?: number; label?: string }) {
  const interval = options?.interval ?? 10_000; // 10s default
  const label = options?.label ?? type;
  const key = `${CACHE_PREFIX}${type}_${id}`;
  const dataRef = useRef<T | null>(null);

  // Load cached draft on mount
  const loadDraft = useCallback((): T | null => {
    try {
      const raw = localStorage.getItem(key);
      if (!raw) return null;
      const entry: DraftEntry<T> = JSON.parse(raw);
      return entry.data;
    } catch {
      return null;
    }
  }, [key]);

  // Save current data to cache
  const saveDraft = useCallback((data: T) => {
    dataRef.current = data;
    try {
      const entry: DraftEntry<T> = {
        id,
        type,
        label,
        data,
        savedAt: new Date().toISOString(),
      };
      localStorage.setItem(key, JSON.stringify(entry));
    } catch {
      // localStorage full or unavailable
    }
  }, [id, type, label, key]);

  // Clear the draft
  const clearDraft = useCallback(() => {
    dataRef.current = null;
    localStorage.removeItem(key);
  }, [key]);

  // Check if a draft exists
  const hasDraft = useCallback((): boolean => {
    return localStorage.getItem(key) !== null;
  }, [key]);

  // Auto-save on interval
  useEffect(() => {
    const timer = setInterval(() => {
      if (dataRef.current !== null) {
        saveDraft(dataRef.current);
      }
    }, interval);
    return () => clearInterval(timer);
  }, [interval, saveDraft]);

  // Save on beforeunload
  useEffect(() => {
    const handler = () => {
      if (dataRef.current !== null) {
        saveDraft(dataRef.current);
      }
    };
    window.addEventListener('beforeunload', handler);
    return () => window.removeEventListener('beforeunload', handler);
  }, [saveDraft]);

  return { loadDraft, saveDraft, clearDraft, hasDraft };
}

/**
 * List all cached drafts across all types.
 */
export function listDrafts(): Array<{ key: string; type: string; label: string; id: string; savedAt: string }> {
  const drafts: Array<{ key: string; type: string; label: string; id: string; savedAt: string }> = [];
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i);
      if (k && k.startsWith(CACHE_PREFIX)) {
        const raw = localStorage.getItem(k);
        if (raw) {
          const entry = JSON.parse(raw);
          drafts.push({
            key: k,
            type: entry.type ?? 'unknown',
            label: entry.label ?? 'Untitled',
            id: entry.id ?? '',
            savedAt: entry.savedAt ?? '',
          });
        }
      }
    }
  } catch { /* ignore */ }
  return drafts.sort((a, b) => b.savedAt.localeCompare(a.savedAt));
}

/**
 * Delete a specific cached draft.
 */
export function deleteDraft(key: string): void {
  localStorage.removeItem(key);
}
