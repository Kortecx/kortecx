import { Link } from "@tanstack/react-router";

/** The kortecx wordmark + icon, linking home. `compact` shows the icon only. */
export function Brand({ compact = false }: { compact?: boolean }) {
  return (
    <Link to="/" className="brand" aria-label="kortecx home" data-testid="brand">
      <img src="/kortecx-icon.png" alt="" className="brand__icon" width={26} height={26} />
      {compact ? null : <span className="brand__word">kortecx</span>}
    </Link>
  );
}
