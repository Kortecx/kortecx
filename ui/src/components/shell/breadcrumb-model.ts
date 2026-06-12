/**
 * Pure pathname → breadcrumb derivation (no React) — the navbar trail that took
 * the brand's old slot (the logo now lives only in the sidebar). Matches the
 * current pathname against the nav model; an extra path segment (today only
 * `/workflows/$instanceId`) becomes a second crumb, hex-shortened when id-shaped.
 */

import { shortHex } from "../../lib/format";
import { HIDDEN_SECTIONS, NAV_SECTIONS, type RoutePath, SETTINGS_SECTION } from "./nav-model";

export interface Crumb {
  readonly label: string;
  /** Set on ancestor crumbs (rendered as links); absent on the current crumb. */
  readonly path?: RoutePath;
}

// HIDDEN_SECTIONS (deep-link-only routes) still breadcrumb (empty since PR-2).
const ALL_SECTIONS = [...NAV_SECTIONS, ...HIDDEN_SECTIONS, SETTINGS_SECTION];

/** A server-derived id rendered as hex: 16-byte run instance ids (32 chars)
 *  and 32-byte mote/content ids (64 chars). */
const HEX_ID = /^[0-9a-f]{32}$|^[0-9a-f]{64}$/;

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
