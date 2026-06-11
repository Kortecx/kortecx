import { Icon } from "./Icon";
import { NavItem } from "./NavItem";
import { NAV_SECTIONS, SETTINGS_SECTION } from "./nav-model";

/**
 * The persistent section navigation. Header = logo + hamburger (collapse →
 * logo-only rail); Settings is pinned bottom-left (D137); the seven sections
 * scroll in between.
 */
export function Sidebar({ collapsed, onToggle }: { collapsed: boolean; onToggle: () => void }) {
  return (
    <nav
      className={collapsed ? "sidebar sidebar--collapsed" : "sidebar"}
      aria-label="Console sections"
      data-testid="sidebar"
      data-collapsed={collapsed}
    >
      <div className="sidebar__head">
        <span className="sidebar__logo">
          <img src="/kortecx-icon.png" alt="" width={24} height={24} />
          {collapsed ? null : <span className="sidebar__logoword">kortecx</span>}
        </span>
        <button
          type="button"
          className="iconbtn"
          onClick={onToggle}
          aria-label="Toggle sidebar"
          data-testid="sidebar-toggle"
        >
          <Icon name="menu" />
        </button>
      </div>
      {NAV_SECTIONS.map((section) => (
        <NavItem key={section.id} section={section} collapsed={collapsed} />
      ))}
      <div className="sidebar__settings">
        <NavItem section={SETTINGS_SECTION} collapsed={collapsed} />
      </div>
    </nav>
  );
}
