import React from "react";
import { vi } from "vitest";

// Stub framer-motion: render plain DOM elements (motion-only props stripped) so
// component tests are fast + deterministic and the perf budget measures OUR table,
// not framer-motion's jsdom cost. Real animation is exercised by the browser E2E.
// Top-level + harmless when framer-motion isn't imported (e.g. the node contract test).
const MOTION_PROPS = new Set([
  "initial",
  "animate",
  "exit",
  "transition",
  "variants",
  "whileHover",
  "whileTap",
  "whileInView",
  "whileFocus",
  "layout",
  "layoutId",
  "drag",
]);

function strip(props: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const k of Object.keys(props)) {
    if (!MOTION_PROPS.has(k)) {
      out[k] = props[k];
    }
  }
  return out;
}

vi.mock("framer-motion", () => {
  const cache = new Map<string, (props: Record<string, unknown>) => unknown>();
  const motion = new Proxy(
    {},
    {
      get: (_t, tag: string) => {
        if (!cache.has(tag)) {
          cache.set(tag, (props: Record<string, unknown>) =>
            React.createElement(tag, strip(props)),
          );
        }
        return cache.get(tag);
      },
    },
  );
  return {
    motion,
    // The LazyMotion `m.*` components share the same plain-element stub.
    m: motion,
    AnimatePresence: ({ children }: { children: unknown }) => children,
    MotionConfig: ({ children }: { children: unknown }) => children,
    LazyMotion: ({ children }: { children: unknown }) => children,
    domAnimation: {},
  };
});

// DOM-only setup (jsdom). Skipped in the node-environment contract test.
if (typeof document !== "undefined") {
  await import("@testing-library/jest-dom/vitest");
  const { cleanup } = await import("@testing-library/react");
  const { afterEach } = await import("vitest");
  afterEach(() => {
    cleanup();
  });
}
