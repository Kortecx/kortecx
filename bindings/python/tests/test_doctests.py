"""Run the docstring doctests on the PURE, server-free client modules.

The SDK's headline pure surfaces (id encoding, the persona library, the read-model
enums, the typed errors, and the Chains DSL / Flow builder lowering) carry
copy-pasteable ``>>>`` examples. This module executes them under ``pytest`` so a
plain ``python -m pytest`` (or ``python -m pytest --doctest-modules``) proves they
stay correct — none of them touch a server, so they run offline and deterministically.

Each module is doctested with ``ELLIPSIS`` + ``IGNORE_EXCEPTION_DETAIL`` so the
exception-raising examples match on the exception TYPE without pinning a volatile
message string.
"""

from __future__ import annotations

import doctest
import importlib

import pytest

# The pure, server-free modules whose public functions carry doctests.
DOCTEST_MODULES = [
    "kortecx.hexids",
    "kortecx.personas",
    "kortecx.types",
    "kortecx.errors",
    "kortecx.chains",
    "kortecx.flow",
]

_OPTIONFLAGS = doctest.ELLIPSIS | doctest.IGNORE_EXCEPTION_DETAIL


@pytest.mark.parametrize("module_name", DOCTEST_MODULES)
def test_module_doctests(module_name: str) -> None:
    module = importlib.import_module(module_name)
    results = doctest.testmod(module, optionflags=_OPTIONFLAGS, verbose=False)
    assert results.attempted > 0, f"{module_name} has no doctests to run"
    assert results.failed == 0, f"{module_name}: {results.failed} doctest failure(s)"


def test_doctest_coverage_is_nonzero() -> None:
    """Guard: the doctest set actually exercises examples (catches an accidental
    empty-docstring regression across the whole pure surface)."""
    total = 0
    for name in DOCTEST_MODULES:
        module = importlib.import_module(name)
        total += doctest.testmod(module, optionflags=_OPTIONFLAGS, verbose=False).attempted
    assert total >= 12, f"expected the pure modules to carry >= 12 doctest examples, got {total}"
