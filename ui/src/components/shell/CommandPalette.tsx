import { useNavigate } from "@tanstack/react-router";
import { AnimatePresence, m } from "framer-motion";
import { type KeyboardEvent, useEffect, useMemo, useState } from "react";
import { paletteIn } from "../../app/motion";
import { Icon } from "./Icon";
import { HIDDEN_SECTIONS, NAV_SECTIONS, type NavSection, SETTINGS_SECTION } from "./nav-model";

// POC-5c (D168): jump to any section — the eight flat sidebar sections PLUS the
// five demoted-but-reachable routes (Blueprints/Datasets/Branches/Policies/Dashboard,
// {@link HIDDEN_SECTIONS}) — so ⌘K never loses a capability the sidebar no longer lists.
const DESTINATIONS: readonly NavSection[] = [...NAV_SECTIONS, ...HIDDEN_SECTIONS, SETTINGS_SECTION];

function matches(section: NavSection, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (q === "") {
    return true;
  }
  return (
    section.label.toLowerCase().includes(q) ||
    section.hint.toLowerCase().includes(q) ||
    section.id.includes(q)
  );
}

/**
 * The ⌘K command palette — jump to any console section. A custom modal (no cmdk
 * dependency, mirroring the reference's hand-rolled dialog): substring filter,
 * ↑/↓ + Enter keyboard navigation, Esc/backdrop to close. The global ⌘K listener
 * lives in the AppShell (palette state is shell chrome).
 */
export function CommandPalette({ open, onClose }: { open: boolean; onClose: () => void }) {
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);

  const results = useMemo(() => DESTINATIONS.filter((s) => matches(s, query)), [query]);
  const selected = results[Math.min(cursor, Math.max(results.length - 1, 0))];

  // Reset the filter every time the palette opens (a fresh jump, not a session).
  useEffect(() => {
    if (open) {
      setQuery("");
      setCursor(0);
    }
  }, [open]);

  function go(section: NavSection): void {
    onClose();
    navigate({ to: section.path });
  }

  function onKeyDown(e: KeyboardEvent<HTMLInputElement>): void {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setCursor((c) => Math.min(c + 1, results.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setCursor((c) => Math.max(c - 1, 0));
    } else if (e.key === "Enter" && selected) {
      e.preventDefault();
      go(selected);
    } else if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  }

  return (
    <AnimatePresence>
      {open ? (
        <>
          <m.div
            className="palette__backdrop"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.12 }}
            onClick={onClose}
            aria-hidden="true"
          />
          <m.div
            className="palette"
            // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride AnimatePresence exit animations; dialog semantics are declared via role+aria-modal
            role="dialog"
            aria-modal="true"
            aria-label="Command palette"
            data-testid="command-palette"
            initial={paletteIn.initial}
            animate={paletteIn.animate}
            exit={paletteIn.exit}
            transition={paletteIn.transition}
          >
            <input
              className="palette__input"
              placeholder="Jump to a section…"
              value={query}
              onChange={(e) => {
                setQuery(e.target.value);
                setCursor(0);
              }}
              onKeyDown={onKeyDown}
              // biome-ignore lint/a11y/noAutofocus: a command palette focuses its input by design
              autoFocus
              spellCheck={false}
              autoComplete="off"
              aria-label="Search sections"
            />
            {results.length === 0 ? (
              <p className="palette__empty">No matching section.</p>
            ) : (
              <nav className="palette__list" aria-label="Sections">
                {results.map((section, i) => (
                  <button
                    type="button"
                    key={section.id}
                    className="palette__item"
                    data-selected={section === selected}
                    data-testid={`palette-item-${section.id}`}
                    onClick={() => go(section)}
                    onMouseEnter={() => setCursor(i)}
                  >
                    <Icon name={section.icon} size={16} />
                    <span>{section.label}</span>
                    <span className="palette__hint">{section.hint}</span>
                  </button>
                ))}
              </nav>
            )}
          </m.div>
        </>
      ) : null}
    </AnimatePresence>
  );
}
