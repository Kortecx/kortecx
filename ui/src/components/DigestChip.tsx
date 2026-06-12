import { useState } from "react";
import { shortHex } from "../lib/format";

/**
 * A content-address chip: the truncated hex is the secondary affordance (the
 * resolved content is always the headline, B5), click copies the FULL hex, the
 * title shows it. Applied wherever a 32-byte ref meets the user (chat
 * attachments now; runs/edges/datasets as those sections grow).
 */
export function DigestChip({ hex, label }: { hex: string; label?: string }) {
  const [copied, setCopied] = useState(false);

  function copy(): void {
    void navigator.clipboard
      ?.writeText(hex)
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1200);
      })
      .catch(() => {
        /* clipboard unavailable (permissions/insecure context) — the title still shows the full hex */
      });
  }

  return (
    <button
      type="button"
      className="digestchip mono"
      onClick={copy}
      title={`${label ? `${label}: ` : ""}${hex} — click to copy`}
      data-testid="digest-chip"
    >
      {copied ? "copied" : shortHex(hex)}
    </button>
  );
}
