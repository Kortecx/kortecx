<!--
  Kortecx OSS pull request template. Every checkbox below corresponds to a
  standing engineering practice for this repo. Tick boxes only
  for items genuinely satisfied; leave the rest unchecked so the reviewer
  knows what's still open.

  CI runs `just ci` + smoke tests. The other items are honor-system +
  reviewer sanity check.
-->

## Summary

<!-- One paragraph: what this PR changes and why. Reference the P-step
     number if applicable (e.g. P1.7-e.2). -->

## Standing-note compliance

### Sensitive content

- [ ] `gh pr diff` reviewed manually; no secrets, credentials, or
      unintended files in the diff.

### SN-4 v1 — Functionality + determinism

- [ ] **Determinism asserted** where determinism is promised (greedy
      sampling, content-addressed storage, replay, seed-based RNG): the
      relevant test runs the operation twice and asserts identical output.
- [ ] **Full-feature-surface coverage**: optional knobs each have at least
      one assertion-bearing test, not just a "compiles" test.
- [ ] **Integration plumbing**: every config knob mapped to a C-struct
      field has a test proving the value reaches the C side.
- [ ] **Error-variant reachability**: every named error variant is either
      test-reachable OR explicitly documented as deferred.

### SN-4 v2 — Property / concurrency / doctests / architectural review

- [ ] **Property tests** added for any function accepting user-supplied input.
- [ ] **Concurrency tests** for any new type that claims `Send` (spawn ≥ 2
      real threads).
- [ ] **Doctests** on every new non-trivial public method / type.
- [ ] **Architectural review** done; refactor candidates noted in PR
      description (fix-vs-defer recorded).

### SN-6 — Forward-build readiness (NEW P-STEPS ONLY)

If this PR introduces a new P-step crate that depends on existing ones:

- [ ] Dependency-graph audit performed: every imported crate is at SN-4 v2 ✅.
- [ ] If any dependency is below bar, that gap is closed in an intermediate
      `PX.Y-eN` step (not this one).

### SN-7 — Cross-platform verification

- [ ] **(A) Linux x86_64 CI** has passed on this PR (link below).
- [ ] **(B) Apple Silicon local** `cargo test` has been run by the
      maintainer (output snippet below). If the change is doc-only or
      `.github/`-only, write "doc-only / no-code" in the snippet.

#### Linux CI link
<!-- Paste the green Actions URL once available; otherwise "pending". -->

#### Apple Silicon local pass
<!-- Final line of `cargo test --workspace` (and feature-gated tests if
     applicable), e.g.:
     test result: ok. 50 passed; 0 failed; 0 ignored
     OR write "doc-only / no-code" if no Rust changed. -->

## Linked decisions / standing notes

- D-number(s): <!-- e.g. D29, or "none — process change only" -->
- Standing notes invoked: <!-- e.g. SN-4 v2 + SN-7, or "none" -->

## Test plan

<!-- Bulleted checklist of what was tested end-to-end. The reviewer will
     re-run any of these locally if anything looks off. -->

---

🤖 Optional: Generated with [Claude Code](https://claude.com/claude-code)
