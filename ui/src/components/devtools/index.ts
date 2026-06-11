import { lazy } from "react";

/** Lazy entry — the dock's chunk loads only when first opened (never eager). */
export const DevToolsDock = lazy(() =>
  import("./DevToolsDock").then((m) => ({ default: m.DevToolsDock })),
);
