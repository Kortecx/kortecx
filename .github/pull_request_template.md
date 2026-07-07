<!--
  Kortecx pull request template. Tick only the boxes you genuinely satisfy; leave the rest
  so reviewers know what is still open. CI runs `just ci` + smoke tests.
-->

## Summary

<!-- One paragraph: what this PR changes and why. -->

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor / internal cleanup
- [ ] Docs only

## Testing

<!-- Bulleted: what you tested end-to-end. Reviewers may re-run these locally. -->

- [ ] `cargo test --workspace` passes locally.
- [ ] Determinism-sensitive paths (greedy sampling, content-addressed storage, replay,
      seed-based RNG) assert identical output across two runs.
- [ ] New public methods have doctests; new user-supplied-input paths have property tests.

## Checklist

- [ ] `gh pr diff` reviewed; no secrets, credentials, or unintended files in the diff.
- [ ] Linux CI is green (link below); for code changes, macOS (Apple Silicon)
      `cargo test --workspace` was run locally.

#### CI link
<!-- Paste the green Actions URL, or "pending". -->

---

🤖 Optional: Generated with [Claude Code](https://claude.com/claude-code)
