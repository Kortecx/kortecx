# Contributing to kortecx

Contributions are welcome. kortecx is an execution kernel for AI agents — the
guarantees are load-bearing, so the bar for changes to the core is high, but
there is plenty of approachable work, and this guide is meant to get you from
clone to first PR without spelunking.

Read the README's [How it works](README.md#how-it-works) and
[GLOSSARY.md](GLOSSARY.md) first — a 30-minute investment that makes the codebase
legible.

> kortecx is in early development. Interfaces will change before 1.0. Open an
> issue to discuss any substantial change before sending a PR, so we can point
> you at the right seam and flag invariants you'd otherwise have to discover.

## Prerequisites

- **Rust 1.94.0+** (pinned in `rust-toolchain.toml`; `rustup` will honor it).
- For the native inference crate (`kx-llamacpp`): a C++ toolchain + CMake +
  libclang, and the `llama.cpp` submodule.
  - Linux: `sudo apt-get install -y libclang-dev clang cmake build-essential`
  - macOS: Xcode Command Line Tools (`xcode-select --install`)
  - `git submodule update --init --recursive`
- `just check-reproducible`'s byte-determinism check and the inference smoke need
  those native deps; the rest of the workspace builds without them.

Run `just doctor` (if you have [`just`](https://github.com/casey/just)) for a
**tiered** preflight: Tier 0 (Rust only — the FFI-free runtime) vs Tier 1 (the
C++ toolchain + submodule for inference), with the exact per-OS install command
for anything missing.

## Build and test

```bash
git clone https://github.com/Kortecx/kortecx.git
cd kortecx
just setup                 # FFI-free: build + install the `kx` binary (Rust only)
cargo build --workspace
cargo test  --workspace
```

Onboarding shortcuts: `just setup-inference` (opt into the native llama.cpp
backend), `just fetch-demo-model` (download a tiny SHA-256-verified GGUF), and
`just verify-quickstart` (run the README quickstart end to end and assert the
canonical digest — the gate that keeps the docs honest).

To see the authoring path end to end — author a workflow → compile to a Mote DAG
→ run → fold the journal — run the worked example:

```bash
cargo run -p kx-workflow --example author_a_workflow
```

To exercise the core *guarantee* (exactly-once across a crash), run the
crash-and-replay demo in the [README](README.md#install--quick-start).

## The pre-merge gate (run it before you push)

CI **gates the merge** — a PR cannot merge until every required check is green,
and they run *before* merge, not after. Mirror the full gate locally:

```bash
just ci      # fmt-check · clippy -D warnings · test · cargo-deny · doc -D warnings
             # · ffi-link · check-reproducible · scale-smoke
```

If `just` isn't on your PATH, the gate is a handful of cargo invocations:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

The remaining gates (`ffi-link`, `check-reproducible`, `scale-smoke`) are slower;
CI runs them on every PR. `cargo fmt --all` before committing saves a round trip.

## Where to start

The dependency graph is a clean layered DAG (see the README's
[How it works](README.md#how-it-works)). **Touch a leaf or a forward-seam before
you touch the waist.**

- **Approachable:** doc-comments (especially turning a terse invariant into a clear
  explanation), additional unit/property tests, a new runnable example, a leaf
  crate (`kx-critic-types`, `kx-normalizer`), or a forward-seam crate
  (`kx-tiering`, `kx-capture`).
- **Handle with care (high concept-density, cross-cutting invariants):**
  `kx-journal/src/entry.rs` (the log format + recovery fold), `kx-projection`
  (the pure fold + cascade), `kx-executor/src/{lifecycle,commit_protocol}.rs`
  (the exactly-once spine), `kx-refusal/src/refusal.rs` (the submission gate).
  Changes here want a design discussion in an issue first.
- **Off-limits without a decision:** anything that changes a `JournalEntry`
  encoding, `MoteId` derivation, or the commit protocol changes the on-disk/
  on-the-wire contract. These are versioned, schema-bumped, and reviewed
  carefully — open an issue.

## How we keep the core trustworthy (working norms)

These are the practices the project holds itself to; new contributions are
reviewed against them. They're not bureaucracy — each one prevents a class of
bug that corrupts the log or breaks a guarantee.

1. **Interface mindfulness, asymmetric strictness.** Maximum care on the six trait
   seams + the Mote/journal core (an honest signature there is load-bearing for
   both the local and the future hosted implementation); pragmatic inside a crate.
2. **Warnings are signal.** Don't silence a warning — especially on the lifecycle/
   journal/recovery paths, where a warning is often the visible edge of an
   ordering or lifecycle bug. Understand it, fix the cause, add a test.
3. **Thin `lib.rs`.** `lib.rs` is module declarations + re-exports; real code lives
   in sibling modules, one concern per file. New crates are born this way.
4. **CI gates the merge.** It runs before merge; keep `just ci` green.
5. **Choose the right type/structure.** Make illegal states unrepresentable
   (enums over bool-soup, typed ids, honest `Result` errors); incremental over
   re-compute on hot paths; deterministic structures on the journal/content path.
6. **Adopt better crates at the point of use** — not speculatively, and always
   clearing `cargo-deny` (permissive licenses only).
7. **Pre-flight every change.** Before implementing, reason about — and surface in
   the PR — what could break if it ships, what could degrade performance, what
   could open a security hole, and how the change makes the runtime better. Then
   present the risks and recommend the best option. No silent decisions; no
   compromise on reliability or efficiency.

A structural refactor is its own PR — never bundled with a feature. Keep diffs
focused so the change says exactly what it does.

## Pull requests

- Branch from `main`, keep the PR scoped to one concern, and write a clear title
  + body (what, why, how it was tested).
- Public items get doc-comments. Behavior changes get tests (unit/property; and,
  for anything on the guarantee path, a failure/recovery test).
- Be ready for review focused on the invariants above. We'd rather discuss a seam
  change up front than unwind it later.

## Working in parallel

Many changes can land at the same time. To keep parallel PRs conflict-free:

- **One branch per concern, off `main`.** Short-lived branches that land in hours,
  not one long-lived branch per whole feature. A `git worktree` per task keeps
  concurrent work path-disjoint.
- **Keep PRs additive and small.** Additive proto is fine — the `proto-breaking`
  check keeps the wire backward-compatible; the execution kernel is frozen — the
  `frozen-trio` check fails a PR that edits it (a deliberate kernel change updates
  that guard in the same PR). Additive, kernel-untouching PRs don't conflict on the
  runtime's core invariants and can be reviewed and merged independently.
- **The merge queue is the integration point.** Once your PR is approved and green,
  it enters the merge queue, which re-tests it against the latest `main` plus the
  PRs ahead of it — so two PRs that are each green alone but break together are
  caught *before* they land, not after. Nothing to do by hand; just approve + queue.
- **Prefer independent branches over stacked PRs.** If a change genuinely depends on
  another's code, say so; otherwise branch each from `main`. If you do stack, retarget
  the dependent PR's base to `main` *before* the base PR merges (a merged-and-deleted
  base auto-closes its dependents).
- **Land incomplete work behind a flag.** Prefer many small PRs that land dark (behind
  an off-by-default flag) over one large PR that must land all at once.

The required checks (build/test, `clippy`, `fmt`, `cargo-deny`, `frozen-trio`,
`proto-breaking`, the real-model gate, and the private-content leak check) are the
contract: green ⇒ safe to merge. If a check is red, fix the cause — never bypass it.

## A note on the design corpus

The full design rationale (the long-form "why" behind individual decisions) is
maintained in a private corpus; the public repo carries the code, the public
doc-comments, and these guides. For contribution you should not need the private
material — the public doc-comments on the core types are intended to be
self-contained. **If a public doc-comment is unclear, references something you
can't find, or an invariant isn't explained where you'd expect it, that's a
documentation bug** — open an issue and we'll fix the public docs. Improving that
public legibility is itself some of the most valuable contribution you can make.

## License

By contributing you agree your contributions are licensed under the
[Apache License, Version 2.0](LICENSE).
