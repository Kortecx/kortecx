/**
 * Pure display formatting. The runtime's ids are 16B/32B lowercase hex strings
 * (server-derived, SN-8); we only ever shorten them for display, never compute one.
 */

/** Shorten a long hex id to `head…tail` (returns the input unchanged if short). */
export function shortHex(hex: string, head = 8, tail = 4): string {
  if (hex.length <= head + tail + 1) {
    return hex;
  }
  return `${hex.slice(0, head)}…${hex.slice(-tail)}`;
}

/** Render an optional journal sequence as `#<n>` (or an em dash when absent). */
export function formatSeq(seq: number | null | undefined): string {
  return seq == null ? "—" : `#${seq}`;
}

/** A stable count summary like "3/5 committed". */
export function countSummary(done: number, total: number, noun: string): string {
  return `${done}/${total} ${noun}`;
}
