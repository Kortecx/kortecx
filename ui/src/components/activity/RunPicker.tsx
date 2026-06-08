import { type FormEvent, useState } from "react";
import { useRuns } from "../../kx/use-runs";
import { shortHex } from "../../lib/format";

const INSTANCE_HEX = /^[0-9a-f]{32}$/;

/** Choose which run the Activity view follows — from session history or by id. */
export function RunPicker({
  selected,
  onSelect,
}: {
  selected?: string;
  onSelect: (instanceId: string) => void;
}) {
  const { runs } = useRuns();
  const [manual, setManual] = useState("");
  const manualValid = INSTANCE_HEX.test(manual.trim());

  function openManual(e: FormEvent<HTMLFormElement>): void {
    e.preventDefault();
    const v = manual.trim();
    if (INSTANCE_HEX.test(v)) {
      onSelect(v);
    }
  }

  return (
    <div className="run-picker" data-testid="run-picker">
      <label htmlFor="run-select">Run</label>
      <select
        id="run-select"
        value={selected ?? ""}
        onChange={(e) => {
          if (e.target.value) {
            onSelect(e.target.value);
          }
        }}
      >
        <option value="">— select a run —</option>
        {runs.map((r) => (
          <option key={r.instanceId} value={r.instanceId}>
            {shortHex(r.instanceId)} · {r.handle ?? "run"}
          </option>
        ))}
      </select>
      <form className="run-picker__manual" onSubmit={openManual}>
        <input
          value={manual}
          onChange={(e) => setManual(e.target.value)}
          placeholder="or paste an instance id (32 hex)"
          spellCheck={false}
          autoComplete="off"
          aria-label="Instance id"
        />
        <button type="submit" className="linkbtn" disabled={!manualValid}>
          Open
        </button>
      </form>
    </div>
  );
}
