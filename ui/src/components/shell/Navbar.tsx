import { Brand } from "./Brand";
import { ConnectionStatus } from "./ConnectionStatus";
import { GlobalControls } from "./GlobalControls";
import { Icon } from "./Icon";
import { SearchTrigger } from "./SearchTrigger";

/**
 * The top bar: brand · ⌘K search trigger · devtools toggle · global controls ·
 * connection status. The sidebar hamburger lives in the Sidebar header (D137).
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
      <Brand />
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
