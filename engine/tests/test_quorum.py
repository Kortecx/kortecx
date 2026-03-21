"""Tests for the Quorum multi-agent orchestration engine."""

from __future__ import annotations

import asyncio
from unittest.mock import AsyncMock
from uuid import UUID, uuid4

import pytest

from engine.services.quorum.errors import (
    ExecutionError,
    InferenceError,
    QuorumError,
    SchedulerError,
    ValidationError,
)
from engine.services.quorum.executor import PipelineExecutor
from engine.services.quorum.types import (
    AgentOutput,
    CompletionResponse,
    MetricsSnapshot,
    Operation,
    RunFilter,
    RunRequest,
    RunResult,
)

# ── Subtask Parsing Tests ────────────────────────────────────────────────────


class TestParseSubtasks:
    """Test the PipelineExecutor._parse_subtasks static method."""

    def test_parses_numbered_list(self):
        text = "1. First subtask\n2. Second subtask\n3. Third subtask"
        result = PipelineExecutor._parse_subtasks(text, 3)
        assert result == ["First subtask", "Second subtask", "Third subtask"]

    def test_parses_numbered_list_with_parentheses(self):
        text = "1) First subtask\n2) Second subtask\n3) Third subtask"
        result = PipelineExecutor._parse_subtasks(text, 3)
        assert result == ["First subtask", "Second subtask", "Third subtask"]

    def test_pads_if_too_few_subtasks(self):
        text = "1. Only one subtask"
        result = PipelineExecutor._parse_subtasks(text, 3)
        assert len(result) == 3
        assert result[0] == "Only one subtask"
        assert result[1].startswith("Continue working on:")
        assert result[2].startswith("Continue working on:")

    def test_truncates_if_too_many_subtasks(self):
        text = "1. Analyze the data\n2. Build the model\n3. Evaluate results\n4. Write report\n5. Present findings"
        result = PipelineExecutor._parse_subtasks(text, 3)
        assert len(result) == 3
        assert result == ["Analyze the data", "Build the model", "Evaluate results"]

    def test_handles_empty_text(self):
        text = ""
        result = PipelineExecutor._parse_subtasks(text, 2)
        assert len(result) == 2
        for subtask in result:
            assert subtask.startswith("Continue working on:")

    def test_skips_short_lines(self):
        text = "1. Good subtask here\n2. \n3. Another good one"
        result = PipelineExecutor._parse_subtasks(text, 2)
        assert len(result) == 2
        assert result[0] == "Good subtask here"
        assert result[1] == "Another good one"

    def test_handles_mixed_formatting(self):
        text = "1. First task\n\nSome explanation text\n2. Second task\n3. Third task"
        result = PipelineExecutor._parse_subtasks(text, 3)
        assert len(result) == 3
        assert result[0] == "First task"

    def test_strips_whitespace(self):
        text = "  1.  Subtask with spaces  \n  2.  Another one  "
        result = PipelineExecutor._parse_subtasks(text, 2)
        assert result[0] == "Subtask with spaces"
        assert result[1] == "Another one"


# ── Response Validation Tests ────────────────────────────────────────────────


class TestValidateResponse:
    """Test the PipelineExecutor._validate_response static method."""

    def test_valid_response(self):
        assert PipelineExecutor._validate_response("This is a valid response with good content.") is True

    def test_empty_response(self):
        assert PipelineExecutor._validate_response("") is False

    def test_whitespace_only(self):
        assert PipelineExecutor._validate_response("   \n\t  ") is False

    def test_too_short(self):
        assert PipelineExecutor._validate_response("Hi") is False
        assert PipelineExecutor._validate_response("    ") is False

    def test_error_prefix(self):
        assert PipelineExecutor._validate_response("Error: something went wrong") is False
        assert PipelineExecutor._validate_response("error: connection refused") is False

    def test_fatal_prefix(self):
        assert PipelineExecutor._validate_response("Fatal: out of memory") is False
        assert PipelineExecutor._validate_response("FATAL: crash") is False

    def test_five_char_boundary(self):
        assert PipelineExecutor._validate_response("12345") is True
        assert PipelineExecutor._validate_response("1234") is False


# ── Pydantic Model Tests ────────────────────────────────────────────────────


class TestRunRequest:
    """Test RunRequest pydantic model validation."""

    def test_defaults(self):
        req = RunRequest(task="Analyze this data")
        assert req.project == "default"
        assert req.task == "Analyze this data"
        assert req.model == "llama3.2:3b"
        assert req.backend == "ollama"
        assert req.workers == 3
        assert req.system_prompt == ""
        assert req.temperature == 0.7
        assert req.max_tokens == 2048
        assert req.retries == 3
        assert req.config is None

    def test_custom_values(self):
        req = RunRequest(
            project="myproject",
            task="Write code",
            model="mistral:7b",
            backend="llamacpp",
            workers=5,
            system_prompt="You are helpful.",
            temperature=0.3,
            max_tokens=4096,
            retries=5,
            config={"key": "value"},
        )
        assert req.project == "myproject"
        assert req.workers == 5
        assert req.backend == "llamacpp"
        assert req.config == {"key": "value"}

    def test_worker_bounds(self):
        with pytest.raises(Exception):
            RunRequest(task="test", workers=0)
        with pytest.raises(Exception):
            RunRequest(task="test", workers=65)

    def test_temperature_bounds(self):
        with pytest.raises(Exception):
            RunRequest(task="test", temperature=-0.1)
        with pytest.raises(Exception):
            RunRequest(task="test", temperature=2.1)

    def test_retries_bounds(self):
        with pytest.raises(Exception):
            RunRequest(task="test", retries=0)
        with pytest.raises(Exception):
            RunRequest(task="test", retries=11)


class TestRunResult:
    """Test RunResult pydantic model."""

    def test_create_result(self):
        result = RunResult(
            id="abc-123",
            project="default",
            task="Do something",
            status="complete",
            backend="ollama",
            model="llama3.2:3b",
            workers=3,
            total_tokens=500,
            total_duration_ms=10000,
            decompose_ms=1000,
            execute_ms=7000,
            synthesize_ms=2000,
            final_output="Here is the final answer.",
            workers_succeeded=3,
            workers_failed=0,
            workers_recovered=0,
        )
        assert result.status == "complete"
        assert result.total_tokens == 500
        assert result.workers_succeeded == 3

    def test_defaults(self):
        result = RunResult(
            id="x", project="p", task="t", status="s",
            backend="b", model="m", workers=1,
        )
        assert result.total_tokens == 0
        assert result.final_output == ""
        assert result.error is None


class TestAgentOutput:
    """Test AgentOutput pydantic model."""

    def test_success_output(self):
        out = AgentOutput(
            agent_id="worker_1",
            subtask="Do thing",
            content="Result here",
            tokens_used=100,
            duration_ms=500,
            attempt=1,
            status="success",
        )
        assert out.status == "success"
        assert out.error is None

    def test_failed_output(self):
        out = AgentOutput(
            agent_id="worker_2",
            subtask="Do other thing",
            status="failed",
            error="Connection refused",
        )
        assert out.status == "failed"
        assert out.error == "Connection refused"
        assert out.content == ""


class TestOperation:
    """Test Operation pydantic model."""

    def test_create(self):
        uid = uuid4()
        op = Operation(
            run_id=uid,
            agent_id="master",
            phase="decompose",
            operation="decompose",
            prompt="Break this down",
            response="1. A\n2. B",
            tokens_used=50,
            duration_ms=200,
        )
        assert op.run_id == uid
        assert op.status == "ok"
        assert op.error is None


class TestRunFilter:
    """Test RunFilter validation."""

    def test_defaults(self):
        f = RunFilter()
        assert f.project is None
        assert f.status is None
        assert f.limit == 50
        assert f.offset == 0

    def test_limit_bounds(self):
        with pytest.raises(Exception):
            RunFilter(limit=0)
        with pytest.raises(Exception):
            RunFilter(limit=1001)


class TestMetricsSnapshot:
    """Test MetricsSnapshot model."""

    def test_defaults(self):
        m = MetricsSnapshot()
        assert m.cpu_usage == 0.0
        assert m.active_runs == 0


class TestCompletionResponse:
    """Test CompletionResponse model."""

    def test_create(self):
        r = CompletionResponse(text="hello", tokens_used=10, model="llama3", duration_ms=100)
        assert r.text == "hello"
        assert r.tokens_used == 10


# ── Error Hierarchy Tests ────────────────────────────────────────────────────


class TestErrorHierarchy:
    """Test the custom exception hierarchy."""

    def test_base_error(self):
        err = QuorumError("something broke", details={"key": "val"})
        assert str(err) == "something broke"
        assert err.details == {"key": "val"}

    def test_inference_error_is_quorum_error(self):
        err = InferenceError("model not found")
        assert isinstance(err, QuorumError)

    def test_scheduler_error_is_quorum_error(self):
        err = SchedulerError("queue full")
        assert isinstance(err, QuorumError)

    def test_execution_error_is_quorum_error(self):
        err = ExecutionError("phase failed")
        assert isinstance(err, QuorumError)

    def test_validation_error_is_quorum_error(self):
        err = ValidationError("bad input")
        assert isinstance(err, QuorumError)

    def test_default_details(self):
        err = QuorumError("test")
        assert err.details == {}


# ── Scheduler Tests (with mocks) ────────────────────────────────────────────


class TestSchedulerSubmitAndStatus:
    """Test scheduler submit and status tracking with mocked dependencies."""

    @pytest.fixture()
    def mock_deps(self):
        db = AsyncMock()
        db.create_run = AsyncMock(return_value={})
        db.update_run = AsyncMock()
        db.connect = AsyncMock()
        db.close = AsyncMock()
        db.migrate = AsyncMock()

        executor = AsyncMock()
        broadcaster = AsyncMock()

        return db, executor, broadcaster

    @pytest.mark.asyncio
    async def test_submit_creates_registry_entry(self, mock_deps):
        from engine.services.quorum.scheduler import Scheduler

        db, executor, broadcaster = mock_deps
        scheduler = Scheduler(executor, db, broadcaster, max_concurrent=2)

        request = RunRequest(task="Test task", project="test_project", workers=2)
        run_id = await scheduler.submit(request)

        assert isinstance(run_id, UUID)
        status = scheduler.status(run_id)
        assert status is not None
        assert status["status"] == "queued"
        assert status["project"] == "test_project"
        assert status["workers"] == 2

    @pytest.mark.asyncio
    async def test_submit_calls_db_create(self, mock_deps):
        from engine.services.quorum.scheduler import Scheduler

        db, executor, broadcaster = mock_deps
        scheduler = Scheduler(executor, db, broadcaster)

        request = RunRequest(task="Another task")
        run_id = await scheduler.submit(request)

        db.create_run.assert_called_once_with(request, run_id)

    @pytest.mark.asyncio
    async def test_submit_broadcasts_queued(self, mock_deps):
        from engine.services.quorum.scheduler import Scheduler

        db, executor, broadcaster = mock_deps
        scheduler = Scheduler(executor, db, broadcaster)

        request = RunRequest(task="Broadcast test")
        run_id = await scheduler.submit(request)

        broadcaster.assert_called_once()
        call_args = broadcaster.call_args
        assert call_args[0][0] == run_id
        assert call_args[0][1] == "quorum.run.queued"

    @pytest.mark.asyncio
    async def test_list_runs_filter_by_project(self, mock_deps):
        from engine.services.quorum.scheduler import Scheduler

        db, executor, broadcaster = mock_deps
        scheduler = Scheduler(executor, db, broadcaster)

        await scheduler.submit(RunRequest(task="Task A", project="alpha"))
        await scheduler.submit(RunRequest(task="Task B", project="beta"))

        f = RunFilter(project="alpha")
        runs = scheduler.list_runs(f)
        assert len(runs) == 1
        assert runs[0]["project"] == "alpha"

    @pytest.mark.asyncio
    async def test_metrics_returns_correct_counts(self, mock_deps):
        from engine.services.quorum.scheduler import Scheduler

        db, executor, broadcaster = mock_deps
        scheduler = Scheduler(executor, db, broadcaster, max_concurrent=8)

        await scheduler.submit(RunRequest(task="Task 1"))
        await scheduler.submit(RunRequest(task="Task 2"))

        m = scheduler.metrics()
        assert m["queued_runs"] == 2
        assert m["active_runs"] == 0
        assert m["max_concurrent"] == 8

    @pytest.mark.asyncio
    async def test_status_returns_none_for_unknown(self, mock_deps):
        from engine.services.quorum.scheduler import Scheduler

        db, executor, broadcaster = mock_deps
        scheduler = Scheduler(executor, db, broadcaster)

        assert scheduler.status(uuid4()) is None


# ── Executor Cancel Check ────────────────────────────────────────────────────


class TestCancelCheck:
    """Test the cancel event check in PipelineExecutor."""

    def test_check_cancelled_raises_when_set(self):
        event = asyncio.Event()
        event.set()
        with pytest.raises(asyncio.CancelledError):
            PipelineExecutor._check_cancelled(event)

    def test_check_cancelled_passes_when_not_set(self):
        event = asyncio.Event()
        PipelineExecutor._check_cancelled(event)  # Should not raise
