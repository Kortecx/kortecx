import { useTheme } from "../../app/use-theme";
import { Breadcrumb } from "./Breadcrumb";
import { ConnectionStatus } from "./ConnectionStatus";
import { GlobalControls } from "./GlobalControls";
import { Icon } from "./Icon";
import { SearchTrigger } from "./SearchTrigger";

/**
 * The top bar (the inverted-L's horizontal arm — starts where the sidebar ends):
 * breadcrumb · ⌘K search trigger · activity drawer · devtools toggle · global
 * controls · theme switch · connection status. The brand lives ONLY in the
 * sidebar (the navbar duplicate was a bug); the sidebar hamburger lives in the
 * Sidebar header (D137).
 */
export function Navbar({
  onOpenPalette,
  onToggleActivity,
  onToggleDevtools,
}: {
  onOpenPalette: () => void;
  onToggleActivity: () => void;
  onToggleDevtools: () => void;
}) {
  const { resolved, setPreference } = useTheme();
  const nextTheme = resolved === "dark" ? "light" : "dark";
  return (
    <header className="navbar" data-testid="navbar">
      <Breadcrumb />
      <div className="navbar__spacer" />
      <SearchTrigger onOpen={onOpenPalette} />
      <div className="navbar__spacer" />
      <button
        type="button"
        className="iconbtn"
        onClick={onToggleActivity}
        title="Activity — live events, metrics & time-travel"
        aria-label="Toggle activity"
        data-testid="activity-toggle"
      >
        <Icon name="activity" />
      </button>
      <button
        type="button"
        className="iconbtn"
        onClick={onToggleDevtools}
        aria-label="Toggle DevTools"
        data-testid="devtools-toggle"
      >
        <Icon name="terminal" />
      </button>
      <GlobalControls />
      <button
        type="button"
        className="iconbtn"
        onClick={() => setPreference(nextTheme)}
        title={`Switch to the ${nextTheme} theme`}
        aria-label={`Switch to the ${nextTheme} theme`}
        data-testid="theme-toggle"
      >
        <Icon name={resolved === "dark" ? "sun" : "moon"} />
      </button>
      <ConnectionStatus />
    </header>
  );
}
