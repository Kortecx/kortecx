"""Quorum pipeline executor — 3+1 phase decompose/execute/recover/synthesize pipeline."""

from __future__ import annotations

import asyncio
import logging
import re
import time
from collections.abc import Awaitable, Callable
from typing import Any
from uuid import UUID

from engine.services.quorum.db import QuorumDB
from engine.services.quorum.errors import ExecutionError
from engine.services.quorum.inference import QuorumInferenceClient
from engine.services.quorum.types import (
    AgentOutput,
    CompletionResponse,
    Operation,
    RunRequest,
    RunResult,
)

logger = logging.getLogger("engine.quorum.executor")

# ── Prompt Templates ─────────────────────────────────────────────────────────

DECOMPOSE_PROMPT = """You are a master coordinator agent.
{system}
Break the following task into exactly {workers} independent subtasks.
Each subtask must be a clear, self-contained work item.

Respond with ONLY a numbered list:
1. <subtask>
2. <subtask>
...

Task: {task}"""

WORKER_PROMPT = """You are worker agent #{N} in a team of {total}.
{system}
Complete the following subtask thoroughly.
Focus on your assigned subtask.

Subtask: {subtask}"""

RECOVERY_PROMPT = """You are a master coordinator agent.
Worker agent '{workerID}' failed while executing its subtask.

Subtask: {subtask}
Error: {errorMessage}

Analyze the error and provide a corrected, improved version of the subtask response.
Complete the subtask yourself since the worker could not.
Provide a thorough response to the subtask:"""

SYNTHESIZE_PROMPT = """You are a master coordinator agent performing final review and synthesis.
{system}
Original task: {task}

Below are outputs from {workers} worker agents.
{error_context}
Your job:
1. Review each worker's output for correctness and quality.
2. Identify gaps, errors, or inconsistencies.
3. Combine into a single, coherent, polished final response.
4. Improve where needed — add missing details, fix errors.
5. If any worker failed and was recovered, ensure the recovered content is integrated properly.

Worker outputs:
{collated}
Provide the final, improved, synthesized response:"""


# ── Broadcaster type alias ───────────────────────────────────────────────────

Broadcaster = Callable[[UUID | None, str, dict[str, Any]], Awaitable[None]]


class PipelineExecutor:
    """Executes the 3-phase quorum pipeline: decompose -> execute -> synthesize.

    Each phase broadcasts real-time events over WebSocket for frontend consumption.
    All operations are logged asynchronously to the quorum_operations table.
    """

    def __init__(self, inference: QuorumInferenceClient, db: QuorumDB, broadcaster: Broadcaster) -> None:
        self._inference = inference
        self._db = db
        self._broadcast = broadcaster

    async def execute(self, run_id: UUID, request: RunRequest, cancel_event: asyncio.Event) -> RunResult:
        """Execute the full 3+1 phase pipeline. Returns a RunResult on success."""
        started = time.monotonic()
        decompose_ms = 0
        execute_ms = 0
        synthesize_ms = 0

        try:
            # Phase 1: DECOMPOSE
            phase_start = time.monotonic()
            subtasks = await self._decompose(run_id, request, cancel_event)
            decompose_ms = int((time.monotonic() - phase_start) * 1000)

            # Save decompose memory
            self._db.log_operation(
                Operation(
                    run_id=run_id,
                    agent_id="master",
                    phase="decompose",
                    operation="phase_complete",
                    tokens_used=0,
                    duration_ms=decompose_ms,
                    status="ok",
                    metadata={"subtasks": subtasks},
                )
            )
            await self._db.save_shared_memory(run_id, "decompose", {"subtasks": subtasks})

            # Phase 2: EXECUTE (parallel workers)
            phase_start = time.monotonic()
            outputs = await self._execute_workers(run_id, request, subtasks, cancel_event)
            execute_ms = int((time.monotonic() - phase_start) * 1000)

            # Phase 2b: RECOVERY (for failed workers)
            outputs = await self._recover_failed(run_id, request, outputs, cancel_event)

            # Save execute memory
            await self._db.save_shared_memory(
                run_id,
                "execute",
                {
                    "outputs": [o.model_dump() for o in outputs],
                    "execute_ms": execute_ms,
                },
            )

            # Phase 3: SYNTHESIZE
            phase_start = time.monotonic()
            final = await self._synthesize(run_id, request, outputs, cancel_event)
            synthesize_ms = int((time.monotonic() - phase_start) * 1000)

            total_ms = int((time.monotonic() - started) * 1000)

            # Compute summary counters
            succeeded = sum(1 for o in outputs if o.status == "success")
            failed = sum(1 for o in outputs if o.status == "failed")
            recovered = sum(1 for o in outputs if o.status == "recovered")
            total_tokens = sum(o.tokens_used for o in outputs) + final.tokens_used

            return RunResult(
                id=str(run_id),
                project=request.project,
                task=request.task,
                status="complete",
                backend=request.backend,
                model=request.model,
                workers=request.workers,
                total_tokens=total_tokens,
                total_duration_ms=total_ms,
                decompose_ms=decompose_ms,
                execute_ms=execute_ms,
                synthesize_ms=synthesize_ms,
                final_output=final.text,
                workers_succeeded=succeeded,
                workers_failed=failed,
                workers_recovered=recovered,
            )

        except asyncio.CancelledError:
            raise
        except ExecutionError:
            raise
        except Exception as e:
            raise ExecutionError(str(e)) from e

    # ── Phase 1: Decompose ───────────────────────────────────────────────────

    async def _decompose(self, run_id: UUID, request: RunRequest, cancel_event: asyncio.Event) -> list[str]:
        """Master LLM breaks the task into N independent subtasks."""
        self._check_cancelled(cancel_event)

        await self._broadcast(
            run_id,
            "quorum.phase.update",
            {
                "run_id": str(run_id),
                "phase": "decompose",
                "status": "started",
                "detail": f"Breaking task into {request.workers} subtasks",
            },
        )
        await self._broadcast(
            run_id,
            "quorum.agent.thinking",
            {
                "run_id": str(run_id),
                "agent_id": "master",
                "phase": "decompose",
                "reasoning": "Analyzing task to identify independent subtasks",
            },
        )

        system = request.system_prompt or ""
        prompt = DECOMPOSE_PROMPT.format(workers=request.workers, task=request.task, system=system)

        start = time.monotonic()
        result = await self._inference.complete(
            backend=request.backend,
            model=request.model,
            prompt=prompt,
            system=system,
            temperature=request.temperature,
            max_tokens=request.max_tokens,
        )
        duration_ms = int((time.monotonic() - start) * 1000)

        self._db.log_operation(
            Operation(
                run_id=run_id,
                agent_id="master",
                phase="decompose",
                operation="decompose",
                prompt=prompt,
                response=result.text,
                tokens_used=result.tokens_used,
                duration_ms=duration_ms,
                status="ok",
            )
        )

        subtasks = self._parse_subtasks(result.text, request.workers)

        await self._broadcast(
            run_id,
            "quorum.phase.update",
            {
                "run_id": str(run_id),
                "phase": "decompose",
                "status": "complete",
                "wall_clock_ms": duration_ms,
                "subtasks": subtasks,
            },
        )

        return subtasks

    # ── Phase 2: Execute Workers ─────────────────────────────────────────────

    async def _execute_workers(
        self,
        run_id: UUID,
        request: RunRequest,
        subtasks: list[str],
        cancel_event: asyncio.Event,
    ) -> list[AgentOutput]:
        """Spawn parallel worker agents, one per subtask."""
        self._check_cancelled(cancel_event)

        await self._broadcast(
            run_id,
            "quorum.phase.update",
            {
                "run_id": str(run_id),
                "phase": "execute",
                "status": "started",
                "detail": f"Spawning {len(subtasks)} workers",
            },
        )

        phase_start = time.monotonic()

        tasks: list[asyncio.Task[AgentOutput]] = []
        for i, subtask in enumerate(subtasks):
            agent_id = f"worker_{i + 1}"
            await self._broadcast(
                run_id,
                "quorum.agent.created",
                {
                    "run_id": str(run_id),
                    "agent_id": agent_id,
                    "role": "worker",
                    "subtask": subtask,
                    "model": request.model,
                },
            )
            tasks.append(
                asyncio.create_task(
                    self._run_worker(run_id, request, agent_id, subtask, i + 1, len(subtasks), cancel_event),
                    name=f"quorum-worker-{run_id}-{agent_id}",
                )
            )

        raw_outputs = await asyncio.gather(*tasks, return_exceptions=True)

        results: list[AgentOutput] = []
        for i, output in enumerate(raw_outputs):
            if isinstance(output, BaseException):
                results.append(
                    AgentOutput(
                        agent_id=f"worker_{i + 1}",
                        subtask=subtasks[i],
                        content="",
                        tokens_used=0,
                        duration_ms=0,
                        attempt=request.retries,
                        status="failed",
                        error=str(output),
                    )
                )
            else:
                results.append(output)

        wall_clock = int((time.monotonic() - phase_start) * 1000)
        sum_individual = sum(o.duration_ms for o in results)
        speedup = sum_individual / wall_clock if wall_clock > 0 else 1.0

        await self._broadcast(
            run_id,
            "quorum.phase.update",
            {
                "run_id": str(run_id),
                "phase": "execute",
                "status": "complete",
                "wall_clock_ms": wall_clock,
                "sum_individual_ms": sum_individual,
                "speedup": round(speedup, 2),
                "parallel": speedup > 1.3,
            },
        )

        return results

    async def _run_worker(
        self,
        run_id: UUID,
        request: RunRequest,
        agent_id: str,
        subtask: str,
        worker_num: int,
        total: int,
        cancel_event: asyncio.Event,
    ) -> AgentOutput:
        """Execute a single worker with retry logic and backoff."""
        system = request.system_prompt or ""
        prompt = WORKER_PROMPT.format(N=worker_num, total=total, subtask=subtask, system=system)
        last_error = ""

        for attempt in range(1, request.retries + 1):
            self._check_cancelled(cancel_event)

            await self._broadcast(
                run_id,
                "quorum.agent.thinking",
                {
                    "run_id": str(run_id),
                    "agent_id": agent_id,
                    "phase": "execute",
                    "reasoning": f"Working on subtask (attempt {attempt}/{request.retries})",
                },
            )

            start = time.monotonic()
            try:
                result = await self._inference.complete(
                    backend=request.backend,
                    model=request.model,
                    prompt=prompt,
                    system=system,
                    temperature=request.temperature,
                    max_tokens=request.max_tokens,
                )
                duration_ms = int((time.monotonic() - start) * 1000)

                if self._validate_response(result.text):
                    self._db.log_operation(
                        Operation(
                            run_id=run_id,
                            agent_id=agent_id,
                            phase="execute",
                            operation="response",
                            prompt=prompt,
                            response=result.text,
                            tokens_used=result.tokens_used,
                            duration_ms=duration_ms,
                            status="ok",
                        )
                    )

                    output = AgentOutput(
                        agent_id=agent_id,
                        subtask=subtask,
                        content=result.text,
                        tokens_used=result.tokens_used,
                        duration_ms=duration_ms,
                        attempt=attempt,
                        status="success",
                    )

                    await self._broadcast(
                        run_id,
                        "quorum.agent.output",
                        {
                            "run_id": str(run_id),
                            "agent_id": agent_id,
                            "phase": "execute",
                            "tokens_used": result.tokens_used,
                            "duration_ms": duration_ms,
                            "content_preview": result.text[:200],
                            "attempt": attempt,
                            "status": "success",
                        },
                    )

                    return output

                # Invalid response — log and retry
                last_error = "Response failed validation"
                self._db.log_operation(
                    Operation(
                        run_id=run_id,
                        agent_id=agent_id,
                        phase="execute",
                        operation="validation_failed",
                        prompt=prompt,
                        response=result.text,
                        tokens_used=result.tokens_used,
                        duration_ms=duration_ms,
                        status="error",
                        error="Response failed validation",
                    )
                )
                if attempt < request.retries:
                    await asyncio.sleep(1.0 * attempt)

            except asyncio.CancelledError:
                raise
            except Exception as e:
                duration_ms = int((time.monotonic() - start) * 1000)
                last_error = str(e)
                self._db.log_operation(
                    Operation(
                        run_id=run_id,
                        agent_id=agent_id,
                        phase="execute",
                        operation="error",
                        prompt=prompt,
                        tokens_used=0,
                        duration_ms=duration_ms,
                        status="error",
                        error=str(e),
                    )
                )
                if attempt < request.retries:
                    await asyncio.sleep(1.0 * attempt)

        # All retries exhausted
        await self._broadcast(
            run_id,
            "quorum.agent.failed",
            {
                "run_id": str(run_id),
                "agent_id": agent_id,
                "phase": "execute",
                "error": f"All {request.retries} attempts failed: {last_error}",
                "attempts": request.retries,
                "subtask": subtask,
            },
        )

        return AgentOutput(
            agent_id=agent_id,
            subtask=subtask,
            content="",
            tokens_used=0,
            duration_ms=0,
            attempt=request.retries,
            status="failed",
            error=f"All {request.retries} attempts failed: {last_error}",
        )

    # ── Phase 2b: Recovery ───────────────────────────────────────────────────

    async def _recover_failed(
        self,
        run_id: UUID,
        request: RunRequest,
        outputs: list[AgentOutput],
        cancel_event: asyncio.Event,
    ) -> list[AgentOutput]:
        """Master agent diagnoses and recovers any failed workers."""
        failed = [o for o in outputs if o.status == "failed"]
        if not failed:
            return outputs

        self._check_cancelled(cancel_event)

        await self._broadcast(
            run_id,
            "quorum.phase.update",
            {
                "run_id": str(run_id),
                "phase": "recovery",
                "status": "started",
                "detail": f"Recovering {len(failed)} failed workers",
            },
        )

        recovered_outputs = list(outputs)
        for original in failed:
            recovery_id = f"{original.agent_id}_recovery"

            prompt = RECOVERY_PROMPT.format(
                workerID=original.agent_id,
                subtask=original.subtask,
                errorMessage=original.error or "Unknown error",
            )

            start = time.monotonic()
            try:
                result = await self._inference.complete(
                    backend=request.backend,
                    model=request.model,
                    prompt=prompt,
                    temperature=request.temperature,
                    max_tokens=request.max_tokens,
                )
                duration_ms = int((time.monotonic() - start) * 1000)

                if self._validate_response(result.text):
                    idx = recovered_outputs.index(original)
                    recovered_outputs[idx] = AgentOutput(
                        agent_id=recovery_id,
                        subtask=original.subtask,
                        content=result.text,
                        tokens_used=result.tokens_used,
                        duration_ms=duration_ms,
                        attempt=1,
                        status="recovered",
                    )

                    self._db.log_operation(
                        Operation(
                            run_id=run_id,
                            agent_id=recovery_id,
                            phase="recovery",
                            operation="response",
                            prompt=prompt,
                            response=result.text,
                            tokens_used=result.tokens_used,
                            duration_ms=duration_ms,
                            status="ok",
                        )
                    )

                    await self._broadcast(
                        run_id,
                        "quorum.agent.recovered",
                        {
                            "run_id": str(run_id),
                            "agent_id": recovery_id,
                            "original_agent": original.agent_id,
                            "phase": "recovery",
                            "tokens_used": result.tokens_used,
                            "duration_ms": duration_ms,
                            "content_preview": result.text[:200],
                        },
                    )
                else:
                    self._db.log_operation(
                        Operation(
                            run_id=run_id,
                            agent_id=recovery_id,
                            phase="recovery",
                            operation="validation_failed",
                            prompt=prompt,
                            response=result.text,
                            tokens_used=result.tokens_used,
                            duration_ms=duration_ms,
                            status="error",
                            error="Recovery response failed validation",
                        )
                    )

            except asyncio.CancelledError:
                raise
            except Exception as e:
                logger.error("Recovery failed for %s: %s", original.agent_id, e)
                self._db.log_operation(
                    Operation(
                        run_id=run_id,
                        agent_id=recovery_id,
                        phase="recovery",
                        operation="error",
                        prompt=prompt,
                        tokens_used=0,
                        duration_ms=int((time.monotonic() - start) * 1000),
                        status="error",
                        error=str(e),
                    )
                )

        await self._broadcast(
            run_id,
            "quorum.phase.update",
            {"run_id": str(run_id), "phase": "recovery", "status": "complete"},
        )

        return recovered_outputs

    # ── Phase 3: Synthesize ──────────────────────────────────────────────────

    async def _synthesize(
        self,
        run_id: UUID,
        request: RunRequest,
        outputs: list[AgentOutput],
        cancel_event: asyncio.Event,
    ) -> CompletionResponse:
        """Master reviews all worker outputs and synthesizes the final response."""
        self._check_cancelled(cancel_event)

        await self._broadcast(
            run_id,
            "quorum.phase.update",
            {
                "run_id": str(run_id),
                "phase": "synthesize",
                "status": "started",
                "detail": f"Synthesizing {len(outputs)} worker outputs",
            },
        )
        await self._broadcast(
            run_id,
            "quorum.agent.thinking",
            {
                "run_id": str(run_id),
                "agent_id": "master",
                "phase": "synthesize",
                "reasoning": "Reviewing worker outputs for quality and synthesizing final response",
            },
        )

        # Build collated output text
        collated_parts: list[str] = []
        for i, output in enumerate(outputs, 1):
            status_tag = f" [{output.status.upper()}]" if output.status != "success" else ""
            header = f"--- Worker {i} ({output.agent_id}){status_tag} ---"
            content = output.content if output.content else f"[FAILED: {output.error}]"
            collated_parts.append(f"{header}\n{content}")

        collated = "\n\n".join(collated_parts)

        # Build error context if any workers failed
        failed_count = sum(1 for o in outputs if o.status == "failed")
        recovered_count = sum(1 for o in outputs if o.status == "recovered")
        error_context = ""
        if failed_count > 0 or recovered_count > 0:
            parts = []
            if failed_count > 0:
                parts.append(f"{failed_count} worker(s) failed completely")
            if recovered_count > 0:
                parts.append(f"{recovered_count} worker(s) were recovered by the master agent")
            error_context = f"Note: {', '.join(parts)}.\n"

        system = request.system_prompt or ""
        prompt = SYNTHESIZE_PROMPT.format(
            system=system,
            task=request.task,
            workers=len(outputs),
            error_context=error_context,
            collated=collated,
        )

        start = time.monotonic()
        result = await self._inference.complete(
            backend=request.backend,
            model=request.model,
            prompt=prompt,
            system=system,
            temperature=max(request.temperature - 0.1, 0.0),  # slightly lower temp for synthesis
            max_tokens=request.max_tokens,
        )
        duration_ms = int((time.monotonic() - start) * 1000)

        self._db.log_operation(
            Operation(
                run_id=run_id,
                agent_id="master",
                phase="synthesize",
                operation="synthesize",
                prompt=prompt,
                response=result.text,
                tokens_used=result.tokens_used,
                duration_ms=duration_ms,
                status="ok",
            )
        )

        await self._db.save_shared_memory(
            run_id,
            "synthesize",
            {"final_output": result.text, "tokens_used": result.tokens_used},
        )

        await self._broadcast(
            run_id,
            "quorum.phase.update",
            {
                "run_id": str(run_id),
                "phase": "synthesize",
                "status": "complete",
                "wall_clock_ms": duration_ms,
            },
        )

        return result

    # ── Utilities ────────────────────────────────────────────────────────────

    @staticmethod
    def _validate_response(text: str) -> bool:
        """Validate that an LLM response is usable (non-empty, no error prefix)."""
        content = text.strip()
        if not content or len(content) < 5:
            return False
        lower = content.lower()
        if lower.startswith("error:") or lower.startswith("fatal:"):
            return False
        return True

    @staticmethod
    def _parse_subtasks(text: str, expected: int) -> list[str]:
        """Parse a numbered list from LLM output into individual subtasks."""
        lines = text.strip().split("\n")
        subtasks: list[str] = []
        for line in lines:
            cleaned = re.sub(r"^\d+[\.\)]\s*", "", line.strip())
            if cleaned and len(cleaned) > 2:
                subtasks.append(cleaned)

        # Pad with generic subtasks if parsing didn't yield enough
        if len(subtasks) < expected:
            base_task = text[:100].strip()
            while len(subtasks) < expected:
                subtasks.append(f"Continue working on: {base_task}")

        return subtasks[:expected]

    @staticmethod
    def _check_cancelled(cancel_event: asyncio.Event) -> None:
        """Raise CancelledError if the cancel event has been set."""
        if cancel_event.is_set():
            raise asyncio.CancelledError()
