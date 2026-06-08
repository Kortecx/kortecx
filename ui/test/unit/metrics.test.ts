import { describe, expect, it } from "vitest";
import { toProjectionVM } from "../../src/kx/use-projection";
import { asPercent, deriveMetrics, foldMetrics } from "../../src/lib/metrics";
import { mote, projection } from "../mocks/projection-fixtures";

const vm = (motes: Parameters<typeof projection>[0], seq?: number) =>
  toProjectionVM(projection(motes, seq != null ? { currentSeq: seq } : {}));

describe("deriveMetrics", () => {
  it("empty projection → zeros, no NaN", () => {
    const m = deriveMetrics(vm([], 0));
    expect(m.total).toBe(0);
    expect(m.committed).toBe(0);
    expect(m.terminal).toBe(0);
    expect(m.inFlight).toBe(0);
    expect(m.successRate).toBe(0);
    expect(m.failureRate).toBe(0);
    expect(Number.isNaN(m.successRate)).toBe(false);
    expect(m.latencySeqSpan).toBeNull();
    expect(m.byState.committed).toBe(0);
  });

  it("all committed → successRate 1, latency span over committed_seq", () => {
    const m = deriveMetrics(
      vm(
        [
          mote({ stateCode: 3, committedSeq: 2 }),
          mote({ stateCode: 3, committedSeq: 5 }),
          mote({ stateCode: 3, committedSeq: 9 }),
        ],
        9,
      ),
    );
    expect(m.total).toBe(3);
    expect(m.committed).toBe(3);
    expect(m.terminal).toBe(3);
    expect(m.inFlight).toBe(0);
    expect(m.successRate).toBe(1);
    expect(m.failureRate).toBe(0);
    expect(m.byState.committed).toBe(3);
    expect(m.latencySeqSpan).toBe(7); // 9 − 2
    expect(m.currentSeq).toBe(9);
  });

  it("mixed states bucket by tone; in-flight counts pending+scheduled", () => {
    const m = deriveMetrics(
      vm([
        mote({ stateCode: 3, committedSeq: 1 }), // committed
        mote({ stateCode: 4 }), // failed (terminal)
        mote({ stateCode: 2 }), // scheduled (in-flight)
        mote({ stateCode: 1 }), // pending (in-flight)
      ]),
    );
    expect(m.committed).toBe(1);
    expect(m.failed).toBe(1);
    expect(m.terminal).toBe(2); // committed + failed
    expect(m.inFlight).toBe(2); // scheduled + pending
    expect(m.successRate).toBeCloseTo(0.5); // 1 committed / 2 terminal
    expect(m.failureRate).toBeCloseTo(0.5);
    expect(m.byState.scheduled).toBe(1);
    expect(m.byState.pending).toBe(1);
  });

  it("all failed → failureRate 1, successRate 0", () => {
    const m = deriveMetrics(vm([mote({ stateCode: 4 }), mote({ stateCode: 4 })]));
    expect(m.failureRate).toBe(1);
    expect(m.successRate).toBe(0);
  });

  it("only in-flight Motes → terminal 0, rates 0 (divide-by-zero guard)", () => {
    const m = deriveMetrics(vm([mote({ stateCode: 1 }), mote({ stateCode: 2 })]));
    expect(m.terminal).toBe(0);
    expect(m.successRate).toBe(0);
    expect(m.failureRate).toBe(0);
  });

  it("latency span is null with fewer than two committed-with-seq", () => {
    const m = deriveMetrics(vm([mote({ stateCode: 3, committedSeq: 4 })]));
    expect(m.latencySeqSpan).toBeNull();
  });

  it("unknown/unspecified state code → unknown tone bucket", () => {
    const m = deriveMetrics(vm([mote({ stateCode: 0 }), mote({ stateCode: 99 })]));
    expect(m.byState.unknown).toBe(2);
    expect(m.terminal).toBe(0); // unknown is not terminal
  });
});

describe("foldMetrics (concurrent runs)", () => {
  it("aggregates Motes across runs and takes the max frontier", () => {
    const a = vm([mote({ stateCode: 3, committedSeq: 1 })], 3);
    const b = vm([mote({ stateCode: 3, committedSeq: 2 }), mote({ stateCode: 4 })], 7);
    const m = foldMetrics([a, b]);
    expect(m.total).toBe(3);
    expect(m.committed).toBe(2);
    expect(m.failed).toBe(1);
    expect(m.currentSeq).toBe(7);
  });

  it("empty list → zeros", () => {
    const m = foldMetrics([]);
    expect(m.total).toBe(0);
    expect(m.currentSeq).toBe(0);
  });
});

describe("asPercent", () => {
  it("rounds to a whole percent", () => {
    expect(asPercent(0)).toBe("0%");
    expect(asPercent(1)).toBe("100%");
    expect(asPercent(2 / 3)).toBe("67%");
  });
});
