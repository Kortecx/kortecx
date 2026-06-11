import { Breadcrumb } from "./Breadcrumb";
import { ConnectionStatus } from "./ConnectionStatus";
import { GlobalControls } from "./GlobalControls";
import { Icon } from "./Icon";
import { SearchTrigger } from "./SearchTrigger";

/**
 * The top bar: breadcrumb · ⌘K search trigger · devtools toggle · global
 * controls · connection status. The brand lives ONLY in the sidebar (the navbar
 * duplicate was a bug); the sidebar hamburger lives in the Sidebar header (D137).
 */
export function Navbar({
  onOpenPalette,
  onToggleDevtools,
}: {
  onOpenPalette: () => void;
  onToggleDevtools: () => void;
}) {
  return (
    <header className="navbar" data-testid="navbar">
      <Breadcrumb />
      <div className="navbar__spacer" />
      <SearchTrigger onOpen={onOpenPalette} />
      <div className="navbar__spacer" />
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
      <ConnectionStatus />
    </header>
  );
}
