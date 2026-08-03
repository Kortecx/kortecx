import { Icon } from "./Icon";

/** The TopNavbar ⌘K affordance — a pill button that opens the command palette.
 *
 * Uses the console's own glyph set. `Icon.tsx` already carried a `search` path, and this
 * was the ONLY import of an icon library anywhere in the app — so the dependency bought
 * one glyph and cost eager bytes on a budget with almost none left. */
export function SearchTrigger({ onOpen }: { onOpen: () => void }) {
  return (
    <button
      type="button"
      className="search-trigger"
      onClick={onOpen}
      aria-label="Open command palette"
      data-testid="palette-trigger"
    >
      <Icon name="search" size={15} aria-hidden="true" />
      <span className="search-trigger__label">Search…</span>
      <kbd>⌘K</kbd>
    </button>
  );
}
