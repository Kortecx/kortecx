import type { Variants, Transition } from 'framer-motion';

/* ═══════════════════════════════════════════════════════
   Shared Framer-Motion Animation Presets
   ═══════════════════════════════════════════════════════ */

const EASE_OUT_CUBIC = [0.25, 0.46, 0.45, 0.94] as const;

/* ── Entrance Variants ───────────────────────────────── */

export const fadeUp: Variants = {
  hidden: { opacity: 0, y: 16 },
  show:   { opacity: 1, y: 0, transition: { duration: 0.38, ease: EASE_OUT_CUBIC } },
};

export const fadeDown: Variants = {
  hidden: { opacity: 0, y: -10 },
  show:   { opacity: 1, y: 0, transition: { duration: 0.4, ease: 'easeOut' } },
};

export const fadeRight: Variants = {
  hidden: { opacity: 0, x: 10 },
  show:   { opacity: 1, x: 0, transition: { duration: 0.35, ease: 'easeOut' } },
};

export const fadeLeft = {
  hidden: { opacity: 0, x: -10 },
  show:   { opacity: 1, x: 0, transition: { duration: 0.35, ease: 'easeOut' } },
};

/* ── Stagger Container ───────────────────────────────── */

export const stagger = (delay = 0.07): Variants => ({
  hidden: {},
  show:   { transition: { staggerChildren: delay } },
});

/* ── Hover / Tap Interactions ────────────────────────── */

export const hoverLift = {
  whileHover: { y: -2, boxShadow: '0 8px 28px rgba(13,13,13,0.10)' } as const,
  transition: { type: 'spring', stiffness: 400, damping: 30 } as Transition,
};

export const hoverLiftLarge = {
  whileHover: { y: -3, boxShadow: '0 10px 32px rgba(13,13,13,0.10)' } as const,
  whileTap:   { scale: 0.99 } as const,
  transition: { type: 'spring', stiffness: 400, damping: 28 } as Transition,
};

export const tapScale = {
  whileTap: { scale: 0.98 } as const,
};

export const buttonHover = {
  whileHover: { y: -1, scale: 1.02, boxShadow: '0 4px 12px rgba(240,69,0,0.30)' } as const,
  whileTap:   { scale: 0.98, y: 0 } as const,
  transition: { type: 'spring', stiffness: 400, damping: 25 } as Transition,
};

export const filterTab = {
  whileHover: { scale: 1.03 } as const,
  whileTap:   { scale: 0.97 } as const,
  transition: { type: 'spring', stiffness: 500, damping: 30 } as Transition,
};

/* ── Row / List Item Entrance ────────────────────────── */

export const rowEntrance = (index: number, baseDelay = 0.2) => ({
  initial:    { opacity: 0, x: -10 },
  animate:    { opacity: 1, x: 0 },
  transition: { delay: index * 0.06 + baseDelay, duration: 0.3, ease: 'easeOut' as const },
});

/* ── Progress Bar ────────────────────────────────────── */

export const progressBar = (widthPercent: number) => ({
  initial:    { width: 0 },
  animate:    { width: `${widthPercent}%` },
  transition: { duration: 1, ease: 'easeOut' as const, delay: 0.4 },
});

/* ── Modal / Dialog ──────────────────────────────────── */

export const modalOverlay = {
  initial:    { opacity: 0 },
  animate:    { opacity: 1 },
  exit:       { opacity: 0 },
  transition: { duration: 0.2 },
};

export const modalContent = {
  initial:    { opacity: 0, scale: 0.96, y: 8 },
  animate:    { opacity: 1, scale: 1, y: 0 },
  exit:       { opacity: 0, scale: 0.96, y: 8 },
  transition: { duration: 0.25, ease: EASE_OUT_CUBIC },
};

/* ── Empty State ─────────────────────────────────────── */

export const emptyState = {
  initial:    { opacity: 0, y: 12 },
  animate:    { opacity: 1, y: 0 },
  transition: { delay: 0.15, duration: 0.4 },
};
