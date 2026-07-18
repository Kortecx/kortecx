import { Link } from "@tanstack/react-router";
import { Brand } from "./Brand";
import { Icon } from "./Icon";
import { NavItem } from "./NavItem";
import { Popover } from "./Popover";
import { NAV_SECTIONS, SETTINGS_SECTION } from "./nav-model";

/**
 * The persistent section navigation (POC-5c / D168 flat IA). Header = the brand (the
 * console's single logo anchor) + collapse hamburger; a primary "New" flyout (real
 * targets only); the flat sections as plain buttons (no groups, no Coming
 * placeholders); Settings pinned bottom-left. Collapsed → an icon rail (labels drop away).
 */
export function Sidebar({ collapsed, onToggle }: { collapsed: boolean; onToggle: () => void }) {
  // The pending-approvals count now lives on the navbar Approvals BELL (a cross-App
  // concern), not the Apps sidebar item — avoiding a double badge.
  return (
    <nav
      className={collapsed ? "sidebar sidebar--collapsed" : "sidebar"}
      aria-label="Console sections"
      data-testid="sidebar"
      data-collapsed={collapsed}
    >
      <div className="sidebar__head">
        <Brand compact={collapsed} />
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

      <div className="sidebar__new">
        <Popover
          trigger={
            collapsed ? (
              <Icon name="plus" />
            ) : (
              <>
                <Icon name="plus" size={15} />
                <span>New</span>
              </>
            )
          }
          triggerClassName="sidebar__new-btn"
          triggerLabel="Create new"
          triggerTestId="sidebar-new"
          align="left"
          direction="down"
          menuTestId="sidebar-new-menu"
        >
          {(close) => (
            <>
              <Link
                to="/chat"
                role="menuitem"
                className="popover__item"
                data-testid="new-chat"
                onClick={close}
              >
                <Icon name="chat" size={15} />
                <span>New chat</span>
              </Link>
              <Link
                to="/blueprints/new"
                role="menuitem"
                className="popover__item"
                data-testid="new-blueprint"
                onClick={close}
              >
                <Icon name="recipes" size={15} />
                <span>New blueprint</span>
              </Link>
              <Link
                to="/blueprints/new"
                role="menuitem"
                className="popover__item"
                data-testid="new-workflow"
                onClick={close}
              >
                <Icon name="runs" size={15} />
                <span>New workflow</span>
              </Link>
            </>
          )}
        </Popover>
      </div>

      <div className="sidebar__nav" data-testid="sidebar-nav">
        {NAV_SECTIONS.map((section) => (
          <NavItem key={section.id} section={section} collapsed={collapsed} />
        ))}
      </div>

      <div className="sidebar__settings">
        <NavItem section={SETTINGS_SECTION} collapsed={collapsed} />
      </div>
    </nav>
  );
}
