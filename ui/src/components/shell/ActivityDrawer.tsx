import { AnimatePresence, m } from "framer-motion";
import { Suspense, lazy, useEffect, useState } from "react";
import { EmptyState } from "../EmptyState";
import { Icon } from "./Icon";

/**
 * The navbar's ACTIVITY DRAWER — the spec's top-bar activities control. Hosts the
 * run-scoped ActivityPanel (live feed · per-run metrics · time-travel) as a right
 * slide-over, available from EVERY section without leaving the page. Replaces the
 * old /activity sidebar section (D141.1 — one capability, one home). Selection
 * state is drawer-local: closing and reopening starts from the run picker.
 */

// Same lazy chunk the old /activity route used — metrics/feed/scrubber stay code-split.
const ActivityPanel = lazy(() =>
  import("../activity/ActivityPanel").then((mod) => ({ default: mod.ActivityPanel })),
);

export function ActivityDrawer({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [instance, setInstance] = useState<string | undefined>(undefined);
  const [atSeq, setAtSeq] = useState<number | undefined>(undefined);

  useEffect(() => {
    if (!open) {
      return;
    }
    function onKeyDown(e: KeyboardEvent): void {
      if (e.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  return (
    <AnimatePresence>
      {open ? (
        <>
          <m.button
            type="button"
            className="activity-drawer__backdrop"
            aria-label="Close activity"
            onClick={onClose}
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.15 }}
          />
          <m.aside
            className="activity-drawer"
            data-testid="activity-drawer"
            aria-label="Activity"
            initial={{ x: 48, opacity: 0 }}
            animate={{ x: 0, opacity: 1 }}
            exit={{ x: 48, opacity: 0 }}
            transition={{ type: "spring", stiffness: 360, damping: 32 }}
          >
            <div className="activity-drawer__head">
              <Icon name="activity" />
              <span className="activity-drawer__title">Activity</span>
              <button
                type="button"
                className="iconbtn activity-drawer__close"
                onClick={onClose}
                aria-label="Close activity"
                data-testid="activity-close"
              >
                ✕
              </button>
            </div>
            <div className="activity-drawer__body">
              <Suspense fallback={<EmptyState title="Loading…" />}>
                <ActivityPanel
                  instance={instance}
                  atSeq={atSeq}
                  onSelectInstance={(id) => {
                    setInstance(id);
                    setAtSeq(undefined);
                  }}
                  onAtSeq={setAtSeq}
                  onNavigate={onClose}
                />
              </Suspense>
            </div>
          </m.aside>
        </>
      ) : null}
    </AnimatePresence>
  );
}
