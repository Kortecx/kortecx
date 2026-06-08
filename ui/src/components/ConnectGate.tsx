import { Link } from "@tanstack/react-router";

/** Shown by protected screens when the app is not connected to a gateway. */
export function ConnectGate() {
  return (
    <div className="empty-state" data-testid="connect-gate">
      <p className="empty-state__title">Not connected</p>
      <p className="empty-state__detail">Connect to a running gateway to continue.</p>
      <Link to="/connect" className="btnlink">
        Go to Connect
      </Link>
    </div>
  );
}
