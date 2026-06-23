import { Link } from "@tanstack/react-router";
import { Brand } from "./Brand";
import { Icon } from "./Icon";
import { NavItem } from "./NavItem";
import { Popover } from "./Popover";
import { TokenUsageFooter } from "./TokenUsageFooter";
import {
  CLOUD_GROUP_LABEL,
  CLOUD_PLACEHOLDERS,
  DEV_GROUP_LABEL,
  DEV_PLACEHOLDERS,
  NAV_GROUPS,
  NAV_SECTIONS,
  SETTINGS_SECTION,
} from "./nav-model";

/**
 * The persistent section navigation (PR-B / D150 reference-app adoption). Header =
 * the brand (the console's single logo anchor) + collapse hamburger; a primary
 * "New" flyout (real targets only); the eight sections in COLOURED GROUPS
 * (Workspace · Data · Tools · Monitoring · Security) plus an HONEST disabled
 * "Cloud" group (GR15 / D129); a real-or-honest-empty token footer; Settings pinned
 * bottom-left. Collapsed → an icon rail (group labels + footer drop away).
 */
const SECTION_BY_ID = new Map(NAV_SECTIONS.map((s) => [s.id, s]));

export function Sidebar({ collapsed, onToggle }: { collapsed: boolean; onToggle: () => void }) {
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

      <div className="sidebar__groups">
        {NAV_GROUPS.map((group) => (
          <div
            key={group.id}
            className="sidebar__group"
            data-color={group.color}
            data-testid={`nav-group-${group.id}`}
          >
            {collapsed ? null : <p className="sidebar__group-label">{group.label}</p>}
            {group.sectionIds.map((id) => {
              const section = SECTION_BY_ID.get(id);
              return section ? (
                <NavItem key={id} section={section} collapsed={collapsed} color={group.color} />
              ) : null;
            })}
          </div>
        ))}
        <div
          className="sidebar__group"
          data-color="neutral"
          data-testid="nav-group-dev"
          aria-label="Coming soon (in development)"
        >
          {collapsed ? null : <p className="sidebar__group-label">{DEV_GROUP_LABEL}</p>}
          {DEV_PLACEHOLDERS.map((p) => (
            <div
              key={p.id}
              className="navitem navitem--disabled"
              aria-disabled="true"
              data-testid={`dev-${p.id}`}
              title={`${p.label} — in development (coming in the POC roadmap)`}
            >
              <span className="navitem__icon">
                <Icon name={p.icon} />
              </span>
              {collapsed ? null : (
                <>
                  <span className="navitem__label">{p.label}</span>
                  <span className="chip chip--soon">In dev</span>
                </>
              )}
            </div>
          ))}
        </div>
        <div
          className="sidebar__group"
          data-color="neutral"
          data-testid="nav-group-cloud"
          aria-label="Cloud (coming soon)"
        >
          {collapsed ? null : <p className="sidebar__group-label">{CLOUD_GROUP_LABEL}</p>}
          {CLOUD_PLACEHOLDERS.map((p) => (
            <div
              key={p.id}
              className="navitem navitem--disabled"
              aria-disabled="true"
              data-testid={`cloud-${p.id}`}
              title={`${p.label} — a managed Cloud capability`}
            >
              <span className="navitem__icon">
                <Icon name={p.icon} />
              </span>
              {collapsed ? null : (
                <>
                  <span className="navitem__label">{p.label}</span>
                  <span className="chip chip--soon">Cloud</span>
                </>
              )}
            </div>
          ))}
        </div>
      </div>

      {collapsed ? null : <TokenUsageFooter />}
      <div className="sidebar__settings">
        <NavItem section={SETTINGS_SECTION} collapsed={collapsed} />
      </div>
    </nav>
  );
}
