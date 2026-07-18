/**
 * The navbar approvals bell — a nav-bar affordance that surfaces the count of pending
 * HITL approvals (polled every 4s by `useListPendingApprovals`) as an incrementing
 * badge, and opens the right-side approvals drawer on click. Replaces the sidebar
 * Apps-item badge (approvals are a cross-App concern, not an Apps-catalog one).
 */

import { useApprovalsDrawer } from "../../app/approvals-context";
import { useListPendingApprovals } from "../../kx/use-approvals";
import { Icon } from "./Icon";

export function ApprovalsBell() {
  const { count } = useListPendingApprovals();
  const { toggle } = useApprovalsDrawer();
  return (
    <div className="approvals-bell">
      <button
        type="button"
        className="iconbtn"
        data-testid="approvals-bell"
        aria-label={count > 0 ? `Approvals (${count} pending)` : "Approvals"}
        title={count > 0 ? `${count} action(s) awaiting approval` : "Approvals"}
        onClick={toggle}
      >
        <Icon name="bell" />
        {count > 0 ? (
          <span
            className="navitem__badge"
            data-testid="nav-badge-approvals"
            aria-label={`${count} awaiting approval`}
          >
            {count > 99 ? "99+" : count}
          </span>
        ) : null}
      </button>
    </div>
  );
}
