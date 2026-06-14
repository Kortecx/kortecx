import { type ReactNode, useEffect, useId, useRef, useState } from "react";

/**
 * A tiny anchored popover menu (PR-4.1, D-numbered as a new interaction pattern).
 * A trigger button toggles a floating `role="menu"` panel that closes on Escape
 * or an outside click and restores focus to the trigger. NOT the `.node-drawer`
 * (that's a full-height side panel — the wrong affordance for an inline button).
 * Kept dependency-free + tiny (the eager bundle budget).
 *
 * The `children` render-prop receives a `close()` so menu items can dismiss the
 * panel after acting.
 */
export function Popover({
  trigger,
  triggerClassName,
  triggerLabel,
  triggerTestId,
  triggerDisabled,
  align = "left",
  menuTestId,
  children,
}: {
  /** The trigger button's inner content (e.g. an `<Icon />`). */
  trigger: ReactNode;
  triggerClassName?: string;
  /** Accessible label + tooltip for the trigger. */
  triggerLabel: string;
  triggerTestId?: string;
  triggerDisabled?: boolean;
  /** Which edge the panel aligns to the trigger. */
  align?: "left" | "right";
  menuTestId?: string;
  children: (close: () => void) => ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuId = useId();

  useEffect(() => {
    if (!open) {
      return;
    }
    function onDown(e: MouseEvent): void {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    function onKey(e: KeyboardEvent): void {
      if (e.key === "Escape") {
        setOpen(false);
        triggerRef.current?.focus();
      }
    }
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div className="popover" ref={rootRef}>
      <button
        ref={triggerRef}
        type="button"
        className={triggerClassName}
        disabled={triggerDisabled}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        aria-label={triggerLabel}
        title={triggerLabel}
        data-testid={triggerTestId}
        onClick={() => setOpen((o) => !o)}
      >
        {trigger}
      </button>
      {open ? (
        <div
          id={menuId}
          className={`popover__panel popover__panel--${align}`}
          role="menu"
          data-testid={menuTestId}
        >
          {children(() => setOpen(false))}
        </div>
      ) : null}
    </div>
  );
}
