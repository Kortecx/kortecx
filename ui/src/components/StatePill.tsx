import { motion } from "framer-motion";
import { statePulse } from "../app/motion";
import { stateVisual } from "../lib/colors";

/** A Mote state as a colored pill that pulses when the state changes (key remount). */
export function StatePill({ stateCode }: { stateCode: number }) {
  const { label, tone } = stateVisual(stateCode);
  return (
    <motion.span
      key={stateCode}
      className={`pill pill--${tone}`}
      data-testid="state-pill"
      data-tone={tone}
      initial={statePulse.initial}
      animate={statePulse.animate}
      transition={statePulse.transition}
    >
      {label}
    </motion.span>
  );
}
