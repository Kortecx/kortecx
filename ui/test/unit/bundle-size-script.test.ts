// @vitest-environment node
import { describe, expect, it } from "vitest";
// @ts-expect-error — a plain .mjs script (no d.ts); the parser export is unit-tested here.
import { eagerJsUrls } from "../../scripts/check-bundle-size.mjs";

const HTML = `<!doctype html>
<html><head>
  <link rel="modulepreload" crossorigin href="/assets/vendor-react-Ab12Cd34.js">
  <link rel="modulepreload" crossorigin href="/assets/vendor-router-Ef56Gh78.js">
  <link rel="stylesheet" crossorigin href="/assets/index-XyZ.css">
  <script type="module" crossorigin src="/assets/index-Qr90St12.js"></script>
</head><body><div id="root"></div></body></html>`;

describe("check-bundle-size eager-set parser", () => {
  it("collects the module entry + every modulepreload JS, nothing else", () => {
    const urls = eagerJsUrls(HTML).sort();
    expect(urls).toEqual([
      "/assets/index-Qr90St12.js",
      "/assets/vendor-react-Ab12Cd34.js",
      "/assets/vendor-router-Ef56Gh78.js",
    ]);
  });

  it("ignores CSS preloads and dedupes repeats", () => {
    const twice = HTML + HTML;
    expect(eagerJsUrls(twice).length).toBe(3);
    expect(eagerJsUrls(HTML).some((u: string) => u.endsWith(".css"))).toBe(false);
  });

  it("returns empty on a build-less page (the script fails loud)", () => {
    expect(eagerJsUrls("<html><body>no scripts</body></html>")).toEqual([]);
  });
});
