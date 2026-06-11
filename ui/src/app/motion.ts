/**
 * Shared framer-motion variants (single source). Page-level fade/slide for route
 * transitions; a one-shot pulse a {@link StatePill} replays when a Mote's state
 * changes. `prefers-reduced-motion` is honored globally via `<MotionConfig
 * reducedMotion="user">` in the providers.
 */

import type { Transition } from "framer-motion";

export const pageFade = {
  initial: { opacity: 0, y: 8 },
  animate: { opacity: 1, y: 0 },
  exit: { opacity: 0, y: -8 },
  transition: { duration: 0.18 } as Transition,
};

export const statePulse = {
  initial: { scale: 0.85, opacity: 0.5 },
  animate: { scale: 1, opacity: 1 },
  transition: { duration: 0.25 } as Transition,
};

/** The ⌘K command palette entrance (subtle scale+fade; exits symmetrically). */
export const paletteIn = {
  initial: { opacity: 0, scale: 0.98, x: "-50%" },
  animate: { opacity: 1, scale: 1, x: "-50%" },
  exit: { opacity: 0, scale: 0.98, x: "-50%" },
  transition: { duration: 0.12 } as Transition,
};
