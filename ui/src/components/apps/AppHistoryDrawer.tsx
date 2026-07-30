import { HistoryDrawer } from "../HistoryDrawer";
import { HostedRestartButton } from "./HostedControls";

/**
 * The App project HISTORY drawer — the generic {@link HistoryDrawer} bound to
 * the App's project branch, with the App-specific honesty: the lock / live
 * scaffold pre-gates (both refuse restore SERVER-side, so the drawer says why
 * up front instead of letting the confirm fail), and the hosted story (a
 * running hosted app keeps serving its pre-restore files until it is
 * restarted — the success state says so and offers the restart).
 *
 * A thin wrapper on purpose: the props (and every testid) are exactly what
 * shipped before the extraction, so `AppDetailSection` and the unit suite are
 * untouched.
 */
export function AppHistoryDrawer({
  handle,
  locked,
  scaffolding,
  hosted,
  onClose,
}: {
  handle: string;
  locked: boolean;
  scaffolding: boolean;
  hosted: boolean;
  onClose: () => void;
}) {
  return (
    <HistoryDrawer
      handle={handle}
      title="Project history"
      blockedMessage={
        locked
          ? "This App is locked — restore is refused while the lock is held. Unlock the App to restore."
          : scaffolding
            ? "A scaffold is writing this project — restore is refused until it finishes."
            : null
      }
      causeTitles={{
        baseline: "The earliest state this project has a record of",
        create: "The project branch was created",
        advance: "A file in the project was written or changed",
        restore: "An earlier recorded state was restored onto the project",
      }}
      restoredExtra={
        hosted ? " This hosted app keeps serving its previous files until you restart it." : ""
      }
      afterRestore={hosted ? <HostedRestartButton handle={handle} /> : null}
      confirmExtra={
        hosted ? " A running hosted app keeps serving its current files until you restart it." : ""
      }
      emptyState={{
        title: "No recorded history yet",
        detail:
          "This App has no project branch yet. Once its project exists, every change to it is recorded here and any recorded state can be restored.",
      }}
      testIdPrefix="app-history"
      onClose={onClose}
    />
  );
}
