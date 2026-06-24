import { Link } from "@tanstack/react-router";
import { Icon } from "./Icon";
import type { NavSection } from "./nav-model";

/**
 * One sidebar entry — a plain button (POC-5c / D168 flat nav). Collapsed → icon only
 * (the label becomes the tooltip). The active item carries `navitem--active`
 * (high-contrast, AA-locked); there are no per-group colours anymore.
 */
export function NavItem({ section, collapsed }: { section: NavSection; collapsed: boolean }) {
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
      </span>
      {collapsed ? null : <span className="navitem__label">{section.label}</span>}
    </Link>
  );
}
