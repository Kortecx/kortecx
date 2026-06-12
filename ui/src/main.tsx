import { RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { AppProviders } from "./app/providers";
import { applyResolvedTheme } from "./lib/theme";
import { router } from "./router/router";
// Geist (self-hosted woff2 via @fontsource — no runtime CDN; weights the UI uses).
import "@fontsource/geist-sans/400.css";
import "@fontsource/geist-sans/500.css";
import "@fontsource/geist-sans/600.css";
import "@fontsource/geist-sans/700.css";
import "@fontsource/geist-mono/400.css";
import "@fontsource/geist-mono/500.css";
import "@xyflow/react/dist/style.css";
import "./styles/app.css";

// Re-stamp the theme the index.html pre-paint script chose (belt + braces: tests
// and any host page without the inline script still get the right palette).
applyResolvedTheme();

const rootEl = document.getElementById("root");
if (rootEl === null) {
  throw new Error("missing #root element");
}

createRoot(rootEl).render(
  <StrictMode>
    <AppProviders>
      <RouterProvider router={router} />
    </AppProviders>
  </StrictMode>,
);
