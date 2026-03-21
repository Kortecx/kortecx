"""Quorum data models — Pydantic V2 schemas for all quorum structures."""

from __future__ import annotations

from typing import Any
from uuid import UUID

from pydantic import BaseModel, Field

# ── Request / Response Models ────────────────────────────────────────────────


class RunRequest(BaseModel):
    """Submit a new quorum run."""

    project: str = "default"
    task: str
    model: str = "llama3.2:3b"
    backend: str = "ollama"
    workers: int = Field(default=3, ge=1, le=64)
    system_prompt: str = ""
    temperature: float = Field(default=0.7, ge=0.0, le=2.0)
    max_tokens: int = Field(default=2048, ge=1, le=131072)
    retries: int = Field(default=3, ge=1, le=10)
    config: dict[str, Any] | None = None


class RunStatus(BaseModel):
    """Current status of a quorum run."""

    id: str
    project: str
    task: str
    status: str
    workers: int
    progress: float = 0.0
    phase: str = ""
    backend: str = ""
    model: str = ""
    started_at: str | None = None
    created_at: str = ""
    total_tokens: int = 0
    total_duration_ms: int = 0


class RunResult(BaseModel):
    """Completed run result with all metrics."""

    id: str
    project: str
    task: str
    status: str
    backend: str
    model: str
    workers: int
    total_tokens: int = 0
    total_duration_ms: int = 0
    decompose_ms: int = 0
    execute_ms: int = 0
    synthesize_ms: int = 0
    final_output: str = ""
    error: str | None = None
    workers_succeeded: int = 0
    workers_failed: int = 0
    workers_recovered: int = 0
    started_at: str | None = None
    finished_at: str | None = None
    created_at: str = ""


class RunFilter(BaseModel):
    """Filter criteria for listing runs."""

    project: str | None = None
    status: str | None = None
    limit: int = Field(default=50, ge=1, le=1000)
    offset: int = Field(default=0, ge=0)


# ── Agent / Worker Models ───────────────────────────────────────────────────


class AgentOutput(BaseModel):
    """Output from a single worker agent."""

    agent_id: str
    subtask: str
    content: str = ""
    tokens_used: int = 0
    duration_ms: int = 0
    attempt: int = 1
    status: str = "pending"
    error: str | None = None


# ── Operation Logging ────────────────────────────────────────────────────────


class Operation(BaseModel):
    """A single logged operation in the quorum pipeline."""

    run_id: UUID
    agent_id: str
    phase: str
    operation: str
    prompt: str | None = None
    response: str | None = None
    tokens_used: int = 0
    duration_ms: int = 0
    status: str = "ok"
    error: str | None = None
    metadata: dict[str, Any] | None = None


class OpFilter(BaseModel):
    """Filter criteria for listing operations."""

    run_id: UUID | None = None
    agent_id: str | None = None
    phase: str | None = None
    operation: str | None = None
    limit: int = Field(default=100, ge=1, le=5000)
    offset: int = Field(default=0, ge=0)


# ── Metrics ──────────────────────────────────────────────────────────────────


class MetricsSnapshot(BaseModel):
    """System metrics at a point in time."""

    cpu_usage: float = 0.0
    memory_usage_mb: float = 0.0
    active_runs: int = 0
    queued_runs: int = 0
    tokens_per_sec: float = 0.0


# ── Shared Memory ───────────────────────────────────────────────────────────


class SharedMemorySnapshot(BaseModel):
    """Phase memory snapshot for inter-phase communication."""

    run_id: str
    phase: str
    memory: dict[str, Any]
    created_at: str = ""


# ── Project Configuration ───────────────────────────────────────────────────


class ProjectConfig(BaseModel):
    """Project-level configuration for quorum runs."""

    name: str
    config: dict[str, Any] = Field(default_factory=dict)


# ── Phase Events ─────────────────────────────────────────────────────────────


class PhaseUpdate(BaseModel):
    """Phase progress event broadcast over WebSocket."""

    run_id: str
    phase: str
    status: str
    detail: str = ""
    wall_clock_ms: int = 0
    sum_individual_ms: int = 0
    speedup: float = 0.0
    parallel: bool = False


# ── Inference Models ─────────────────────────────────────────────────────────


class CompletionRequest(BaseModel):
    """Inference request for the quorum pipeline."""

    model: str
    prompt: str
    system: str = ""
    temperature: float = Field(default=0.7, ge=0.0, le=2.0)
    max_tokens: int = Field(default=2048, ge=1, le=131072)


class CompletionResponse(BaseModel):
    """Inference response from the backend."""

    text: str
    tokens_used: int = 0
    model: str = ""
    duration_ms: int = 0
