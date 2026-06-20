/**
 * A bundle-safe registry for the process-wide default client (V2a g1).
 *
 * Holds the lazily-resolved default client + an optional FACTORY the Node entry
 * installs (so `flow().run()` / `agent().run()` work zero-config in Node). This
 * module has NO node-only imports (no `node:fs`, no transport), so it is safe in the
 * `web` and `chains` bundles too — there no factory is installed and the resolver
 * returns `undefined` (those surfaces are explicit-client by design). Backed by
 * `globalThis` so one process shares ONE client across the SDK + `_toolserver`
 * bundles (tsup `splitting:false` inlines this module separately into each bundle).
 */

const SINGLETON = Symbol.for("kortecx.defaultClient");
const FACTORY = Symbol.for("kortecx.defaultClientFactory");

interface Holder {
  [SINGLETON]?: unknown;
  [FACTORY]?: () => unknown;
}

function holder(): Holder {
  return globalThis as unknown as Holder;
}

/**
 * Install the lazy factory the Node entry uses to build a default client on first
 * use. Idempotent; a later {@link setDefaultClient} overrides the built instance.
 */
export function setDefaultClientFactory(factory: () => unknown): void {
  holder()[FACTORY] = factory;
}

/** Override (or clear, with `undefined`) the process-wide default client. */
export function setDefaultClient(client: unknown): void {
  holder()[SINGLETON] = client;
}

/**
 * The process-wide default client: a previously-set instance, else one built from
 * the installed factory (Node, lazily), else `undefined` (no zero-config default in
 * the `web` / `chains` bundles, which never install a factory).
 */
export function getDefaultClient(): unknown {
  const h = holder();
  if (h[SINGLETON] !== undefined) return h[SINGLETON];
  const f = h[FACTORY];
  if (f !== undefined) {
    h[SINGLETON] = f();
    return h[SINGLETON];
  }
  return undefined;
}
