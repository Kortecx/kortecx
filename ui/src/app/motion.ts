/**
 * Shared framer-motion variants (single source). Page-level fade/slide for route
 * transitions; a one-shot pulse a {@link StatePill} replays when a Mote's state
 * changes; the reference design-language vocabulary (fadeUp/stagger/hover lifts)
 * the section tiles share. All are plain data — LazyMotion-strict `m.*` safe
 * (spring is a transition type in the core engine, not a `domMax` feature).
 * `prefers-reduced-motion` is honored globally via `<MotionConfig
 * reducedMotion="user">` in the providers.
 */

import type { Transition, Variants } from "framer-motion";

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

const EASE_OUT_CUBIC = [0.25, 0.46, 0.45, 0.94] as const;

/** Card/tile entrance: child of a {@link stagger} container (`hidden`→`show`). */
export const fadeUp: Variants = {
  hidden: { opacity: 0, y: 16 },
  show: { opacity: 1, y: 0, transition: { duration: 0.38, ease: EASE_OUT_CUBIC } },
};

/** Stagger container for grids of {@link fadeUp} children. */
export const stagger = (delay = 0.07): Variants => ({
  hidden: {},
  show: { transition: { staggerChildren: delay } },
});

/** Subtle tile lift on hover (cards in dense grids). */
export const hoverLift = {
  whileHover: { y: -2, boxShadow: "0 8px 28px rgba(13,13,13,0.10)" } as const,
  transition: { type: "spring", stiffness: 400, damping: 30 } as Transition,
};

/** Larger lift + tap feedback for primary catalog tiles. */
export const hoverLiftLarge = {
  whileHover: { y: -3, boxShadow: "0 10px 32px rgba(13,13,13,0.10)" } as const,
  whileTap: { scale: 0.99 } as const,
  transition: { type: "spring", stiffness: 400, damping: 28 } as Transition,
};

/** Primary action button: lift + brand-orange glow. */
export const buttonHover = {
  whileHover: { y: -1, scale: 1.02, boxShadow: "0 4px 12px rgba(240,69,0,0.30)" } as const,
  whileTap: { scale: 0.98, y: 0 } as const,
  transition: { type: "spring", stiffness: 400, damping: 25 } as Transition,
};

/** Cap so long feeds don't tail-lag: rows past this animate together. */
const ROW_ENTRANCE_CAP = 12;

/** Staggered list-row entrance keyed by index (spread onto an `m.li`/`m.div`). */
export const rowEntrance = (index: number, baseDelay = 0.2) => ({
  initial: { opacity: 0, x: -10 },
  animate: { opacity: 1, x: 0 },
  transition: {
    delay: Math.min(index, ROW_ENTRANCE_CAP) * 0.06 + baseDelay,
    duration: 0.3,
    ease: "easeOut",
  } as Transition,
});

/** Animated fill for `.progress` bars (width 0 → the given percent). */
export const progressBar = (widthPercent: number) => ({
  initial: { width: 0 },
  animate: { width: `${widthPercent}%` },
  transition: { duration: 1, ease: "easeOut", delay: 0.4 } as Transition,
});

/** Empty-state placeholder entrance. */
export const emptyState = {
  initial: { opacity: 0, y: 12 },
  animate: { opacity: 1, y: 0 },
  transition: { delay: 0.15, duration: 0.4 } as Transition,
};
