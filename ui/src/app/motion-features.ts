/**
 * The framer-motion feature pack, loaded LAZILY by `<LazyMotion>` (providers.tsx).
 * Keeping this in its own module means the animation engine rides a dynamic chunk —
 * the eager bundle carries only the tiny `m`/`LazyMotion` core. `domAnimation`
 * covers everything we use (tweens, variants, exit animations); nothing needs
 * `domMax` (no drag/layout animations).
 */
export { domAnimation } from "framer-motion";
