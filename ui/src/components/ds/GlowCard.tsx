import { type HTMLMotionProps, m } from "framer-motion";
import type { CSSProperties } from "react";

export interface GlowCardProps extends HTMLMotionProps<"div"> {
  /** Border/glow accent on hover (any CSS color; defaults to the brand orange). */
  glowColor?: string;
  /** Disable the hover lift/glow for purely static cards. */
  hover?: boolean;
}

/**
 * The design-system card (reference `components/ui/GlowCard`): a white surface
 * that lifts slightly and glows in the accent color on hover. Pure presentation —
 * tokens drive every color, framer drives the micro-interaction.
 */
export function GlowCard({
  glowColor = "var(--primary)",
  hover = true,
  className,
  style,
  children,
  ...rest
}: GlowCardProps) {
  const classes = `glow-card${hover ? " glow-card--hover" : ""}${className ? ` ${className}` : ""}`;
  const styles = { ...style, "--glow": glowColor } as CSSProperties;
  return (
    <m.div
      className={classes}
      style={styles}
      whileHover={hover ? { scale: 1.005, y: -1 } : undefined}
      {...rest}
    >
      {children}
    </m.div>
  );
}
