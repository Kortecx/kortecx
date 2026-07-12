/**
 * Decode a Mote's `config_subset["kx.context.items"]` value — the EXACT grounded
 * chunk refs the chat-rag bind layer folds into the answer Mote's identity-bearing
 * config — into displayable citation items. A pure UI mirror of the Rust decoder in
 * `crates/kx-mote/src/context_items.rs` (`decode_context_items`): each item is
 * `u32-le(name.len()) ‖ name bytes ‖ content_ref[32]`, in canonical (encoded) order.
 * A malformed / truncated buffer decodes what it can and stops (fail-soft, never
 * throws) — the sources are advisory (the content store still gates every fetch by
 * ref). This READS an already-committed wire value; it never DEFINES one, so nothing
 * here touches the proto / SDK surface.
 */

/** The `config_subset` key the bind layer folds a run's grounded chunk refs under
 *  (mirrors `kx-mote`'s `CONTEXT_ITEMS_KEY`). A Mote without it is ungrounded. */
export const CONTEXT_ITEMS_KEY = "kx.context.items";

/** One grounded context item: an advisory label + the 64-hex content-store ref of
 *  the chunk the answer was grounded on. The label is display-only; identity is the
 *  ref (the citation key). */
export interface ContextItem {
  readonly label: string;
  readonly ref: string;
}

function toHex(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) {
    s += b.toString(16).padStart(2, "0");
  }
  return s;
}

/**
 * Decode the canonical value into items, in encoded order. Total + panic-free: a
 * truncated buffer decodes the items it can and stops (matching the Rust decoder).
 */
export function decodeContextItems(bytes: Uint8Array): ContextItem[] {
  const out: ContextItem[] = [];
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const decoder = new TextDecoder();
  let i = 0;
  while (i + 4 <= bytes.length) {
    const len = view.getUint32(i, true); // u32-le(name.len())
    i += 4;
    if (i + len + 32 > bytes.length) {
      break; // truncated — stop fail-soft.
    }
    const label = decoder.decode(bytes.subarray(i, i + len));
    i += len;
    const ref = toHex(bytes.subarray(i, i + 32));
    i += 32;
    out.push({ label, ref });
  }
  return out;
}
