import { useState } from "react";

/**
 * A small copy-to-clipboard hook (the {@link DigestChip} logic, generalized):
 * `copy(text)` writes to the clipboard and flips `copied` true for `resetMs`,
 * degrading silently when the clipboard is unavailable (permissions / insecure
 * context — the caller still shows the source text).
 */
export function useCopyToClipboard(resetMs = 1200): {
  copied: boolean;
  copy: (text: string) => void;
} {
  const [copied, setCopied] = useState(false);
  function copy(text: string): void {
    void navigator.clipboard
      ?.writeText(text)
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), resetMs);
      })
      .catch(() => {
        /* clipboard unavailable (permissions/insecure context) — degrade silently */
      });
  }
  return { copied, copy };
}
