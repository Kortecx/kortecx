# Testing & Quality Gate — Documentation

## Overview

Kortecx maintains a comprehensive testing and quality gate pipeline that runs across both services (Python engine and TypeScript frontend). Every push is validated by the pre-push hook, and the full test suite covers types, services, edge cases, and integration points.

---

## Quality Gate Script

**Location:** `scripts/check.sh`

| Mode | Command | What it checks |
|------|---------|----------------|
| `--quick` | `./scripts/check.sh --quick` | TypeScript (`tsc`), ESLint, Ruff check + format |
| `--test` | `./scripts/check.sh --test` | Python pytest, Vitest |
| `--lint` | `./scripts/check.sh --lint` | Lint only (no tests) |
| `(default)` | `./scripts/check.sh` | Everything + `next build` |

### Pre-Push Hook

`.git/hooks/pre-push` runs `--quick` mode before every `git push`. Blocks the push on any failure. Skip with `git push --no-verify` for emergencies only.

### npm Scripts

```bash
npm run check        # quick lint + typecheck
npm run check:full   # everything including build
npm run test         # vitest run
npm run test:coverage # vitest with v8 coverage
npm run typecheck    # tsc --noEmit
npm run lint         # eslint (errors only)
```

---

## Test Suites

### Python Engine (205 tests)

| File | Tests | Coverage | Focus |
|------|-------|----------|-------|
| `test_quorum.py` | 54 | types 100%, errors 100% | Type validation, subtask parsing, response validation, scheduler, prompt templates, edge cases |
| `test_expert_manager.py` | 43 | 99% | Loading, CRUD, per-file versioning, restore, deletion, corrupted JSON, duplicate names |
| `test_step_artifacts.py` | 39 | 97% | Disk persistence, script extraction, async execution, timeouts, edge cases (unicode, 1MB responses, concurrent dirs) |
| `test_execution_audit.py` | 34 | 94% | Disabled/enabled behavior, DB injection, all 6 log operations, async complete/fail, error resilience |
| `test_config.py` | 4 | 100% | Settings defaults, quorum/agent config, env overrides |
| `test_local_inference.py` | 14 | — | Inference backends |
| `test_synthesis.py` | 10 | — | Data synthesis |
| `test_system_stats.py` | 7 | — | System resource monitoring |

### Frontend (36 tests)

| File | Tests | Focus |
|------|-------|-------|
| `unit/helpers.test.ts` | 7 | Utility functions |
| `unit/api-client.test.ts` | 5 | API URL construction, fetcher |
| `unit/settings.test.ts` | 6 | Settings persistence, deep merge, corrupt JSON, feature flags |
| `unit/timezone.test.ts` | 5 | UTC formatting, timezone conversion, static TIMEZONES list, formatTzLabel |
| `unit/intelligence.test.ts` | 11 | Fine-tuning jobs CRUD, model tabs, cloud redirect URL, timezone validation |
| `integration/db.test.ts` | 2 | Schema validation |

---

## Coverage by Service

### Python — Key Modules

| Module | Statements | Missed | Coverage |
|--------|-----------|--------|----------|
| `quorum/types.py` | 82 | 0 | **100%** |
| `quorum/errors.py` | 18 | 0 | **100%** |
| `config.py` | 12 | 0 | **100%** |
| `expert_manager.py` | 171 | 2 | **99%** |
| `step_artifacts.py` | 111 | 3 | **97%** |
| `execution_audit.py` | 109 | 6 | **94%** |

### Missed Lines (intentional)
- Exception handlers in `_load_expert` (corrupted files)
- Bare `except` in `_update_registry` (filesystem errors)
- `TimeoutError` branch in `execute_script` (platform-dependent)
- Internal `logger.error` calls in audit error paths

---

## Edge Cases Tested

### Robustness
- Unicode in slugify (Café, Résumé)
- Special characters in error messages (`"`, `\n`, `\t`)
- Empty responses and prompts
- 1MB+ response handling
- Concurrent directory creation (20 parallel)
- Corrupted JSON in expert definitions
- Missing files and directories
- Duplicate expert names

### Timeout & Failure
- Script execution timeout (2s limit on sleep 30)
- Script error exit codes
- Nonexistent file execution
- Unsupported script types
- Nonexistent expert/version operations

### Data Integrity
- Per-file versioning (content preservation, no-op on unchanged)
- Deep merge of settings (new fields don't break saved data)
- Timezone conversion accuracy (UTC → EST verified)
- Feature flag independence (toggling one doesn't affect others)

### Configuration
- Default values for all settings
- Environment variable overrides (`PORT=9999`)
- Quorum and agent configuration defaults

---

## Lint Configuration

| Language | Tool | Config |
|----------|------|--------|
| TypeScript | `tsc --noEmit` | `tsconfig.json` (strict mode) |
| TypeScript | ESLint 9 | Next.js config + `--quiet` for errors only |
| Python | Ruff | `pyproject.toml` — `target-version = "py311"`, `line-length = 170` |
| Python | Ruff format | Auto-formatting check |

---

## Running Tests Locally

```bash
# All checks (lint + tests)
./scripts/check.sh

# Python only
cd engine && .venv/bin/python -m pytest tests/ -v --cov=engine

# Frontend only
cd frontend && npx vitest run --reporter=verbose

# Frontend with coverage
cd frontend && npx vitest run --coverage
```

---

## Adding New Tests

1. **Python:** Add to `engine/tests/test_*.py`, use `pytest` fixtures (`tmp_path`, `monkeypatch`)
2. **Frontend:** Add to `frontend/tests/unit/*.test.ts`, use `vitest` (`describe`/`it`/`expect`)
4. Run `./scripts/check.sh` to verify everything passes before committing
