import { Brand } from "./Brand";
import { ConnectionStatus } from "./ConnectionStatus";
import { GlobalControls } from "./GlobalControls";
import { SearchTrigger } from "./SearchTrigger";

/**
 * The top bar: brand · ⌘K search trigger · global controls · connection status.
 * The sidebar hamburger lives in the Sidebar header (logo+hamburger, D137).
 */
export function Navbar({ onOpenPalette }: { onOpenPalette: () => void }) {
  return (
    <header className="navbar" data-testid="navbar">
      <Brand />
      <div className="navbar__spacer" />
      <SearchTrigger onOpen={onOpenPalette} />
      <div className="navbar__spacer" />
      <GlobalControls />
      <ConnectionStatus />
    </header>
  );
}
