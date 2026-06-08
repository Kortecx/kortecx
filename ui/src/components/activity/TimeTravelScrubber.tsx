/**
 * Time-travel control: a seq slider over `[0, currentSeq]`. Any explicit position
 * pins a static snapshot (`atSeq`); "Live" clears it and resumes polling. The
 * parent wires `atSeq` into `useProjection({ atSeq })` (a pinned snapshot already
 * auto-pauses the poll). Seq 0 is the empty/initial frontier.
 */
export function TimeTravelScrubber({
  currentSeq,
  atSeq,
  onChange,
}: {
  currentSeq: number;
  atSeq?: number;
  onChange: (seq: number | undefined) => void;
}) {
  const pinned = atSeq != null;
  const value = pinned ? atSeq : currentSeq;
  const max = Math.max(currentSeq, 0);
  return (
    <div className="scrubber" data-testid="time-travel">
      <label htmlFor="seq-range">Time-travel</label>
      <input
        id="seq-range"
        type="range"
        min={0}
        max={max}
        step={1}
        value={Math.min(value, max)}
        onChange={(e) => onChange(Number(e.target.value))}
        aria-label="Journal sequence"
      />
      <span className="mono scrubber__seq" data-testid="scrubber-seq">
        {pinned ? `#${atSeq}` : "live"}
      </span>
      <button
        type="button"
        className="linkbtn"
        disabled={!pinned}
        onClick={() => onChange(undefined)}
        data-testid="scrubber-live"
      >
        Live
      </button>
    </div>
  );
}
