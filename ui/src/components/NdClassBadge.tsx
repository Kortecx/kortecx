import { ndClassVisual } from "../lib/colors";

/** The Mote's nondeterminism class (PURE / READ_ONLY_NONDET / WORLD_MUTATING). */
export function NdClassBadge({ ndClass }: { ndClass: number }) {
  const { label, tone } = ndClassVisual(ndClass);
  return (
    <span
      className={`badge badge--${tone}`}
      data-testid="nd-badge"
      data-tone={tone}
      title={`nd_class: ${label}`}
    >
      {label}
    </span>
  );
}
