import { Search } from "lucide-react";

/** The TopNavbar ⌘K affordance — a pill button that opens the command palette. */
export function SearchTrigger({ onOpen }: { onOpen: () => void }) {
  return (
    <button
      type="button"
      className="search-trigger"
      onClick={onOpen}
      aria-label="Open command palette"
      data-testid="palette-trigger"
    >
      <Search size={15} aria-hidden="true" />
      <span className="search-trigger__label">Search…</span>
      <kbd>⌘K</kbd>
    </button>
  );
}
