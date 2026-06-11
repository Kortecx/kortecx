import type { ToolManifest } from "@kortecx/sdk/web";
import { m } from "framer-motion";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { Badge } from "../ds/Badge";
import { GlowCard } from "../ds/GlowCard";

/** Keep tiles scannable — the full keyword set is advisory metadata, not a list view. */
const KEYWORD_DISPLAY_CAP = 6;

/** Tile accent stripe keyed by the manifest kind (display only). */
function stripeFor(kind: string): string {
  if (kind === "Builtin") return "var(--info)";
  if (kind === "Mcp") return "var(--primary)";
  return "var(--violet)";
}

/**
 * The registered tool manifests as a staggered tile grid. Each tile is a native
 * button that toggles its tool into the bundle sequence (the same action as the
 * composer's chip). Everything shown is ADVISORY (SN-8) — listing a manifest
 * leaks no authority; the broker never reads it.
 */
export function ManifestGrid({
  manifests,
  selected,
  onToggle,
}: {
  manifests: readonly ToolManifest[];
  selected: readonly string[];
  onToggle: (toolId: string) => void;
}) {
  return (
    <m.div
      className="tool-grid"
      data-testid="tool-manifest-grid"
      variants={stagger()}
      initial="hidden"
      animate="show"
    >
      {manifests.map((man) => {
        const active = selected.includes(man.toolId);
        const words = man.keywords.flatMap((k) => k.words).slice(0, KEYWORD_DISPLAY_CAP);
        return (
          <GlowCard
            key={`${man.toolId}@${man.toolVersion}`}
            className={`tool-tile${active ? " tool-tile--active" : ""}`}
            stripe={stripeFor(man.kind)}
            variants={fadeUp}
            {...hoverLift}
          >
            <button
              type="button"
              className="tool-tile__btn"
              data-testid={`tool-tile-${man.toolId}`}
              aria-pressed={active}
              onClick={() => onToggle(man.toolId)}
            >
              <div className="tool-tile__head">
                <span className="status-dot status-dot--online" aria-hidden="true" />
                <span className="tool-tile__name mono">
                  {man.toolId}@{man.toolVersion}
                </span>
                <Badge label={man.kind} color={stripeFor(man.kind)} />
              </div>
              <p className="tool-tile__desc muted">{man.description}</p>
              <div className="tool-keywords">
                {words.map((w) => (
                  <Badge key={w} label={w} color="var(--text-2)" />
                ))}
              </div>
            </button>
          </GlowCard>
        );
      })}
    </m.div>
  );
}
