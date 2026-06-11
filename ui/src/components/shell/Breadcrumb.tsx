import { Link, useRouterState } from "@tanstack/react-router";
import { Fragment } from "react";
import { Icon } from "./Icon";
import { deriveCrumbs } from "./breadcrumb-model";

/**
 * The navbar trail (reference TopNavbar pattern): current section, plus a
 * detail crumb on nested routes. Renders nothing on paths outside the nav
 * model (the login gate has its own header and never mounts this).
 */
export function Breadcrumb() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const crumbs = deriveCrumbs(pathname);
  if (crumbs.length === 0) {
    return null;
  }
  return (
    <nav className="breadcrumb" aria-label="Breadcrumb" data-testid="breadcrumb">
      {crumbs.map((crumb, i) => (
        <Fragment key={crumb.path ?? crumb.label}>
          {i > 0 ? (
            <span className="breadcrumb__sep" aria-hidden="true">
              <Icon name="chevron-right" size={14} />
            </span>
          ) : null}
          {crumb.path ? (
            <Link to={crumb.path} className="breadcrumb__crumb">
              {crumb.label}
            </Link>
          ) : (
            <span className="breadcrumb__crumb breadcrumb__crumb--current" aria-current="page">
              {crumb.label}
            </span>
          )}
        </Fragment>
      ))}
    </nav>
  );
}
