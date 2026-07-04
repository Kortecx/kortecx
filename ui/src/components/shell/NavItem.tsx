import { Link } from "@tanstack/react-router";
import { Icon } from "./Icon";
import type { NavSection } from "./nav-model";

/**
 * One sidebar entry — a plain button (POC-5c / D168 flat nav). Collapsed → icon only
 * (the label becomes the tooltip). The active item carries `navitem--active`
 * (high-contrast, AA-locked); there are no per-group colours anymore.
 *
 * `badge` (RC6a) rides the ICON as a small count bubble (so it shows on the collapsed
 * rail too) — used for pending approvals awaiting an operator decision. Absent or `0`
 * renders nothing (an idle nav is unadorned).
 */
export function NavItem({
  section,
  collapsed,
  badge,
}: {
  section: NavSection;
  collapsed: boolean;
  badge?: number;
}) {
  const showBadge = typeof badge === "number" && badge > 0;
  return (
    <Link
      to={section.path}
      className="navitem"
      activeProps={{ className: "navitem navitem--active" }}
      title={collapsed ? section.label : section.hint}
      data-testid={`nav-${section.id}`}
    >
      <span className="navitem__icon">
        <Icon name={section.icon} />
        {showBadge ? (
          <span
            className="navitem__badge"
            data-testid={`nav-badge-${section.id}`}
            aria-label={`${badge} awaiting approval`}
          >
            {badge > 99 ? "99+" : badge}
          </span>
        ) : null}
      </span>
      {collapsed ? null : <span className="navitem__label">{section.label}</span>}
    </Link>
  );
}
