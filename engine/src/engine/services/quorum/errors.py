"""Quorum custom exception hierarchy."""

from __future__ import annotations


class QuorumError(Exception):
    """Base exception for all quorum-related errors."""

    def __init__(self, message: str, *, details: dict | None = None) -> None:
        super().__init__(message)
        self.details = details or {}


class InferenceError(QuorumError):
    """Raised when an inference backend call fails after retries."""


class SchedulerError(QuorumError):
    """Raised when the run scheduler encounters an unrecoverable error."""


class ExecutionError(QuorumError):
    """Raised when the pipeline executor fails during a run."""


class ValidationError(QuorumError):
    """Raised when input validation fails before execution begins."""
