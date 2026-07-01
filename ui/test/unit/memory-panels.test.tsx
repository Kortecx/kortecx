/**
 * RC5b — the Memories section gains adaptive panels: a stats strip, a CHIP-driven
 * decay panel (preview + reversible apply), a consolidate trigger, and a decayed view
 * with restore. This pins that they render with their stable testids + CHIP controls
 * (never a controlled <select>, which Playwright can't drive), so a future refactor
 * that drops them fails CI here.
 */

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const noopMutation = { mutate: vi.fn(), isPending: false, isError: false, data: undefined };

// Mock the memory hooks so the section renders a populated, decay-aware state without a
// live client.
vi.mock("../../src/kx/use-memory", () => ({
  useMemories: () => ({
    data: [
      {
        memoryId: "aa".repeat(32),
        kind: "episodic",
        text: "the user asked about pricing",
        isDecayed: false,
      },
      {
        memoryId: "bb".repeat(32),
        kind: "semantic",
        text: "an old, decayed fact",
        isDecayed: true,
      },
    ],
    isError: false,
  }),
  useMemoryStats: () => ({
    data: {
      total: 2,
      semantic: 1,
      episodic: 1,
      tombstoned: 1,
      dim: 384,
      embedFingerprint: "embeddinggemma:mean",
    },
    isError: false,
  }),
  useMemoryDecay: () => ({
    data: {
      wouldEvict: 1,
      kept: 1,
      candidates: [
        { memoryId: "cc".repeat(32), ageDays: 120, accessCount: 0, text: "a stale note" },
      ],
    },
    isError: false,
  }),
  useMemoryRecall: () => ({ data: [], isError: false }),
  useStoreMemory: () => noopMutation,
  useForgetMemory: () => noopMutation,
  useApplyDecay: () => noopMutation,
  useRestoreMemory: () => noopMutation,
  useConsolidateMemory: () => noopMutation,
}));

import { MemoriesSection } from "../../src/components/sections/MemoriesSection";

describe("Memories — RC5b adaptive panels", () => {
  it("renders the stats strip, CHIP-driven decay panel, consolidate trigger, and restore", () => {
    const { container } = render(<MemoriesSection />);

    // Stats strip with the decayed count.
    expect(screen.getByTestId("memory-stats-strip")).toHaveTextContent("1 decayed");

    // Decay panel — CHIP presets (NOT a controlled <select>) + preview candidates.
    expect(screen.getByTestId("memory-decay-panel")).toBeInTheDocument();
    expect(screen.getByTestId("memory-decay-ttl-90")).toBeInTheDocument();
    expect(screen.getByTestId("memory-decay-access-1")).toBeInTheDocument();
    expect(screen.getByTestId("memory-decay-summary")).toHaveTextContent("Would evict 1");
    expect(screen.getByTestId("memory-decay-candidate-cccccccc")).toBeInTheDocument();
    // The decay policy is chip-driven — there is NO <select> Playwright can't drive.
    expect(container.querySelector("select")).toBeNull();

    // Consolidate trigger.
    expect(screen.getByTestId("memory-consolidate-trigger")).toBeInTheDocument();

    // The decayed memory (bb…) exposes a Restore control, not Forget.
    expect(screen.getByTestId("memory-restore-bbbbbbbb")).toBeInTheDocument();
    expect(screen.getByTestId("memory-forget-aaaaaaaa")).toBeInTheDocument();
  });
});
