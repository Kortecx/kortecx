/**
 * Pure pathname → breadcrumb derivation (no React) — the navbar trail that took
 * the brand's old slot (the logo now lives only in the sidebar). Matches the
 * current pathname against the nav model; an extra path segment (today only
 * `/runs/$instanceId`) becomes a second crumb, hex-shortened when id-shaped.
 */

import { shortHex } from "../../lib/format";
import { HIDDEN_SECTIONS, NAV_SECTIONS, type RoutePath, SETTINGS_SECTION } from "./nav-model";

export interface Crumb {
  readonly label: string;
  /** Set on ancestor crumbs (rendered as links); absent on the current crumb. */
  readonly path?: RoutePath;
}

// HIDDEN_SECTIONS (deep-link-only routes like /artifacts) still breadcrumb.
const ALL_SECTIONS = [...NAV_SECTIONS, ...HIDDEN_SECTIONS, SETTINGS_SECTION];

/** A server-derived 32-byte id rendered as 64 hex chars (run instance ids). */
const HEX_ID = /^[0-9a-f]{64}$/;

export function deriveCrumbs(pathname: string): Crumb[] {
  const section = ALL_SECTIONS.find(
    (s) => pathname === s.path || pathname.startsWith(`${s.path}/`),
  );
  if (!section) {
    return [];
  }
  const rest = pathname.slice(section.path.length).replace(/^\/+|\/+$/g, "");
  if (!rest) {
    return [{ label: section.label }];
  }
  const segment = decodeURIComponent(rest);
  return [
    { label: section.label, path: section.path },
    { label: HEX_ID.test(segment) ? shortHex(segment) : segment },
  ];
}
