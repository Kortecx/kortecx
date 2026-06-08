import { Brand } from "./Brand";
import { ConnectionStatus } from "./ConnectionStatus";
import { GlobalControls } from "./GlobalControls";
import { Icon } from "./Icon";

/** The top bar: sidebar toggle · brand · global controls · connection status. */
export function Navbar({ onToggleSidebar }: { onToggleSidebar: () => void }) {
  return (
    <header className="navbar" data-testid="navbar">
      <button
        type="button"
        className="iconbtn navbar__toggle"
        onClick={onToggleSidebar}
        aria-label="Toggle sidebar"
        data-testid="sidebar-toggle"
      >
        <Icon name="menu" />
      </button>
      <Brand />
      <div className="navbar__spacer" />
      <GlobalControls />
      <ConnectionStatus />
    </header>
  );
}
