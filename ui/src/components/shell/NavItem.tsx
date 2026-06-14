import { Link } from "@tanstack/react-router";
import type { CSSProperties } from "react";
import { Icon } from "./Icon";
import type { NavSection, SectionColor } from "./nav-model";

/**
 * One sidebar entry. Collapsed → icon only (label becomes the tooltip). When a
 * group `color` is set (PR-B / D150), the item carries the section accent as a
 * `--nav-color` custom property: the colour reads on the ICON + hover/active TINT +
 * an active accent bar, while the LABEL text stays high-contrast (the WCAG-AA lock
 * — the accent palette is below 4.5:1 as small light-theme text). `neutral` (and
 * the no-colour Settings item) keep the default muted styling.
 */
export function NavItem({
  section,
  collapsed,
  color,
}: {
  section: NavSection;
  collapsed: boolean;
  color?: SectionColor;
}) {
  const colored = color !== undefined && color !== "neutral";
  const base = colored ? "navitem navitem--colored" : "navitem";
  const style = colored ? ({ "--nav-color": `var(--${color})` } as CSSProperties) : undefined;
  return (
    <Link
      to={section.path}
      className={base}
      activeProps={{ className: `${base} navitem--active` }}
      style={style}
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
