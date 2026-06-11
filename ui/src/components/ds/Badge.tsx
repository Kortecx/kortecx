export interface BadgeProps {
  label: string;
  /** Text/dot color (any CSS color; the pill background derives from it at ~9%). */
  color?: string;
  /** Show the leading status dot. */
  dot?: boolean;
  /** Pulse the dot (live/active states). */
  pulse?: boolean;
}

/**
 * The design-system status badge (reference `components/ui/Badge`): colored text
 * on a same-hue tint, with an optional (pulsing) glow dot. Zero dependencies.
 */
export function Badge({
  label,
  color = "var(--primary-h)",
  dot = false,
  pulse = false,
}: BadgeProps) {
  return (
    <span
      className="ds-badge"
      style={{ color, background: "color-mix(in srgb, currentColor 9%, transparent)" }}
    >
      {dot ? (
        <span
          className={pulse ? "ds-badge__dot ds-badge__dot--pulse" : "ds-badge__dot"}
          aria-hidden="true"
        />
      ) : null}
      {label}
    </span>
  );
}
