"""Execution audit — persists all orchestrator operations to quorum DB for auditing."""

from __future__ import annotations

import logging
from datetime import UTC, datetime
from typing import Any
from uuid import UUID, uuid4

from engine.services.quorum.types import Operation

logger = logging.getLogger("engine.execution_audit")


class ExecutionAudit:
    """Bridges the orchestrator with quorum DB for full audit trail."""

    def __init__(self) -> None:
        self._db: Any = None  # Set during startup via set_db()
        self._enabled = False
        self._run_map: dict[str, UUID] = {}

    def set_db(self, db: Any) -> None:
        """Inject the quorum DB instance after startup."""
        from engine.services.quorum.db import QuorumDB

        if isinstance(db, QuorumDB):
            self._db = db
            self._enabled = True
            logger.info("Execution audit enabled — persisting to quorum DB")

    async def create_run(
        self,
        run_id: str,
        workflow_name: str,
        steps: list[Any],
        **kwargs: Any,
    ) -> UUID | None:
        """Create a quorum_runs record for a workflow execution."""
        if not self._enabled:
            return None
        try:
            from engine.services.quorum.types import RunRequest

            req = RunRequest(
                project=kwargs.get("project", "workflows"),
                task=f"Workflow: {workflow_name}",
                model=kwargs.get("model", "local"),
                backend=kwargs.get("backend", "ollama"),
                workers=len(steps),
                system_prompt=kwargs.get("system_prompt", ""),
            )
            db_run_id = uuid4()
            await self._db.create_run(req, db_run_id)
            self._run_map[run_id] = db_run_id
            return db_run_id
        except Exception as e:
            logger.error("Failed to create audit run: %s", e)
            return None

    def log_agent_spawned(
        self,
        run_id: str,
        agent_id: str,
        step_id: str,
        **kwargs: Any,
    ) -> None:
        """Log agent creation to quorum_operations."""
        if not self._enabled:
            return
        db_run_id = self._run_map.get(run_id)
        if not db_run_id:
            return
        self._db.log_operation(
            Operation(
                run_id=db_run_id,
                agent_id=agent_id,
                phase="execute",
                operation="agent_created",
                status="ok",
                metadata={
                    "stepId": step_id,
                    "expertId": kwargs.get("expert_id"),
                    "modelSource": kwargs.get("model_source"),
                    "role": kwargs.get("role", "worker"),
                    "subtask": kwargs.get("task_description", ""),
                },
            )
        )

    def log_agent_thinking(
        self,
        run_id: str,
        agent_id: str,
        step_id: str,
    ) -> None:
        """Log agent thinking state."""
        if not self._enabled:
            return
        db_run_id = self._run_map.get(run_id)
        if not db_run_id:
            return
        self._db.log_operation(
            Operation(
                run_id=db_run_id,
                agent_id=agent_id,
                phase="execute",
                operation="thinking",
                prompt=f"Processing step {step_id}",
                status="ok",
            )
        )

    def log_inference(
        self,
        run_id: str,
        agent_id: str,
        system_prompt: str,
        user_prompt: str,
        response: str,
        tokens_used: int,
        duration_ms: float,
        status: str = "ok",
        error: str = "",
        **kwargs: Any,
    ) -> None:
        """Log the actual inference call with full prompts and response."""
        if not self._enabled:
            return
        db_run_id = self._run_map.get(run_id)
        if not db_run_id:
            return
        self._db.log_operation(
            Operation(
                run_id=db_run_id,
                agent_id=agent_id,
                phase="execute",
                operation="response",
                prompt=f"[SYSTEM]\n{system_prompt}\n\n[USER]\n{user_prompt}",
                response=response,
                tokens_used=tokens_used,
                duration_ms=int(duration_ms),
                status=status,
                error=error,
                metadata={
                    "model": kwargs.get("model"),
                    "engine": kwargs.get("engine"),
                    "temperature": kwargs.get("temperature"),
                    "maxTokens": kwargs.get("max_tokens"),
                    "fallback": kwargs.get("fallback", False),
                },
            )
        )

    def log_step_complete(
        self,
        run_id: str,
        agent_id: str,
        step_id: str,
        tokens_used: int,
        duration_ms: float,
    ) -> None:
        """Log step completion."""
        if not self._enabled:
            return
        db_run_id = self._run_map.get(run_id)
        if not db_run_id:
            return
        self._db.log_operation(
            Operation(
                run_id=db_run_id,
                agent_id=agent_id,
                phase="execute",
                operation="step_complete",
                tokens_used=tokens_used,
                duration_ms=int(duration_ms),
                status="ok",
                metadata={"stepId": step_id},
            )
        )

    def log_step_failed(
        self,
        run_id: str,
        agent_id: str,
        step_id: str,
        error: str,
    ) -> None:
        """Log step failure."""
        if not self._enabled:
            return
        db_run_id = self._run_map.get(run_id)
        if not db_run_id:
            return
        self._db.log_operation(
            Operation(
                run_id=db_run_id,
                agent_id=agent_id,
                phase="execute",
                operation="step_failed",
                status="error",
                error=error,
                metadata={"stepId": step_id},
            )
        )

    async def complete_run(
        self,
        run_id: str,
        total_tokens: int,
        total_duration_ms: int,
        final_output: str,
        workers_succeeded: int,
        workers_failed: int,
    ) -> None:
        """Mark run as complete with aggregated stats."""
        if not self._enabled:
            return
        db_run_id = self._run_map.get(run_id)
        if not db_run_id:
            return
        try:
            await self._db.update_run(
                db_run_id,
                status="complete",
                finished_at=datetime.now(UTC),
                total_tokens=total_tokens,
                total_duration_ms=total_duration_ms,
                final_output=final_output[:10000],
                workers_succeeded=workers_succeeded,
                workers_failed=workers_failed,
            )
        except Exception as e:
            logger.error("Failed to complete audit run: %s", e)

    async def fail_run(self, run_id: str, error: str) -> None:
        """Mark run as failed."""
        if not self._enabled:
            return
        db_run_id = self._run_map.get(run_id)
        if not db_run_id:
            return
        try:
            await self._db.update_run(
                db_run_id,
                status="failed",
                finished_at=datetime.now(UTC),
                error=error,
            )
        except Exception as e:
            logger.error("Failed to mark audit run as failed: %s", e)

    async def save_shared_memory(
        self,
        run_id: str,
        phase: str,
        memory: dict[str, Any],
    ) -> None:
        """Persist shared memory snapshot."""
        if not self._enabled:
            return
        db_run_id = self._run_map.get(run_id)
        if not db_run_id:
            return
        try:
            await self._db.save_shared_memory(db_run_id, phase, memory)
        except Exception as e:
            logger.error("Failed to save shared memory: %s", e)

    async def get_run_operations(self, run_id: str) -> list[dict[str, Any]]:
        """Get all operations for a run (for audit trail viewer)."""
        if not self._enabled:
            return []
        db_run_id = self._run_map.get(run_id)
        if not db_run_id:
            return []
        try:
            from engine.services.quorum.types import OpFilter

            return await self._db.list_operations(OpFilter(run_id=db_run_id))
        except Exception as e:
            logger.error("Failed to get run operations: %s", e)
            return []


execution_audit = ExecutionAudit()
