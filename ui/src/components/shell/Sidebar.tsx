import { NavItem } from "./NavItem";
import { NAV_SECTIONS } from "./nav-model";

/** The persistent section navigation (collapsible to an icon rail). */
export function Sidebar({ collapsed }: { collapsed: boolean }) {
  return (
    <nav
      className={collapsed ? "sidebar sidebar--collapsed" : "sidebar"}
      aria-label="Console sections"
      data-testid="sidebar"
      data-collapsed={collapsed}
    >
      {NAV_SECTIONS.map((section) => (
        <NavItem key={section.id} section={section} collapsed={collapsed} />
      ))}
    </nav>
  );
}
