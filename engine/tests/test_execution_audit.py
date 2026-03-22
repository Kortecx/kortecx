"""Tests for execution audit service."""

from __future__ import annotations

import pytest
from unittest.mock import AsyncMock, MagicMock, patch
from uuid import uuid4

from engine.services.execution_audit import ExecutionAudit
from engine.services.quorum.db import QuorumDB
from engine.services.quorum.types import Operation, RunRequest


class TestExecutionAuditDisabled:
    def test_disabled_by_default(self):
        audit = ExecutionAudit()
        assert not audit._enabled

    def test_db_is_none_by_default(self):
        audit = ExecutionAudit()
        assert audit._db is None

    def test_run_map_empty_by_default(self):
        audit = ExecutionAudit()
        assert audit._run_map == {}

    def test_log_agent_spawned_noop(self):
        audit = ExecutionAudit()
        audit.log_agent_spawned("run-1", "agent-1", "step-1")

    def test_log_agent_thinking_noop(self):
        audit = ExecutionAudit()
        audit.log_agent_thinking("run-1", "agent-1", "step-1")

    def test_log_inference_noop(self):
        audit = ExecutionAudit()
        audit.log_inference("run-1", "agent-1", "sys", "user", "resp", 100, 500)

    def test_log_step_complete_noop(self):
        audit = ExecutionAudit()
        audit.log_step_complete("run-1", "agent-1", "step-1", 100, 500)

    def test_log_step_failed_noop(self):
        audit = ExecutionAudit()
        audit.log_step_failed("run-1", "agent-1", "step-1", "error")

    @pytest.mark.asyncio
    async def test_complete_run_noop(self):
        audit = ExecutionAudit()
        await audit.complete_run("run-1", 100, 1000, "output", 3, 0)

    @pytest.mark.asyncio
    async def test_fail_run_noop(self):
        audit = ExecutionAudit()
        await audit.fail_run("run-1", "error")

    @pytest.mark.asyncio
    async def test_save_shared_memory_noop(self):
        audit = ExecutionAudit()
        await audit.save_shared_memory("run-1", "phase", {})

    @pytest.mark.asyncio
    async def test_get_run_operations_returns_empty(self):
        audit = ExecutionAudit()
        result = await audit.get_run_operations("run-1")
        assert result == []

    @pytest.mark.asyncio
    async def test_create_run_returns_none(self):
        audit = ExecutionAudit()
        result = await audit.create_run("run-1", "Test Workflow", [1, 2, 3])
        assert result is None


def _make_enabled_audit():
    """Create an audit instance with a mocked QuorumDB."""
    audit = ExecutionAudit()
    mock_db = MagicMock(spec=QuorumDB)
    mock_db.create_run = AsyncMock()
    mock_db.update_run = AsyncMock()
    mock_db.save_shared_memory = AsyncMock()
    mock_db.list_operations = AsyncMock(return_value=[])
    mock_db.log_operation = MagicMock()
    # Bypass isinstance check by setting __class__
    mock_db.__class__ = QuorumDB
    audit.set_db(mock_db)
    return audit, mock_db


class TestExecutionAuditEnabled:
    def test_set_db_enables(self):
        audit, _ = _make_enabled_audit()
        assert audit._enabled

    def test_set_db_non_quorum_does_not_enable(self):
        audit = ExecutionAudit()
        audit.set_db("not a QuorumDB")
        assert not audit._enabled

    @pytest.mark.asyncio
    async def test_create_run(self):
        audit, mock_db = _make_enabled_audit()
        result = await audit.create_run("run-1", "Test Workflow", [1, 2, 3])
        assert result is not None
        mock_db.create_run.assert_called_once()

    @pytest.mark.asyncio
    async def test_create_run_stores_mapping(self):
        audit, mock_db = _make_enabled_audit()
        await audit.create_run("run-1", "Test Workflow", [1, 2])
        assert "run-1" in audit._run_map

    @pytest.mark.asyncio
    async def test_create_run_error_returns_none(self):
        audit, mock_db = _make_enabled_audit()
        mock_db.create_run = AsyncMock(side_effect=Exception("DB error"))
        result = await audit.create_run("run-1", "Test", [])
        assert result is None

    def test_log_agent_spawned_calls_db(self):
        audit, mock_db = _make_enabled_audit()
        db_run_id = uuid4()
        audit._run_map["run-1"] = db_run_id
        audit.log_agent_spawned("run-1", "agent-1", "step-1")
        mock_db.log_operation.assert_called_once()
        op = mock_db.log_operation.call_args[0][0]
        assert op.run_id == db_run_id
        assert op.operation == "agent_created"

    def test_log_agent_spawned_no_mapping_noop(self):
        audit, mock_db = _make_enabled_audit()
        audit.log_agent_spawned("unmapped-run", "agent-1", "step-1")
        mock_db.log_operation.assert_not_called()

    def test_log_agent_thinking_calls_db(self):
        audit, mock_db = _make_enabled_audit()
        db_run_id = uuid4()
        audit._run_map["run-1"] = db_run_id
        audit.log_agent_thinking("run-1", "agent-1", "step-1")
        mock_db.log_operation.assert_called_once()
        op = mock_db.log_operation.call_args[0][0]
        assert op.operation == "thinking"

    def test_log_inference_calls_db(self):
        audit, mock_db = _make_enabled_audit()
        db_run_id = uuid4()
        audit._run_map["run-1"] = db_run_id
        audit.log_inference("run-1", "agent-1", "sys", "user", "resp", 100, 500)
        mock_db.log_operation.assert_called_once()
        op = mock_db.log_operation.call_args[0][0]
        assert op.operation == "response"
        assert op.tokens_used == 100

    def test_log_step_complete_calls_db(self):
        audit, mock_db = _make_enabled_audit()
        db_run_id = uuid4()
        audit._run_map["run-1"] = db_run_id
        audit.log_step_complete("run-1", "agent-1", "step-1", 200, 1000)
        mock_db.log_operation.assert_called_once()
        op = mock_db.log_operation.call_args[0][0]
        assert op.operation == "step_complete"
        assert op.tokens_used == 200

    def test_log_step_failed_calls_db(self):
        audit, mock_db = _make_enabled_audit()
        db_run_id = uuid4()
        audit._run_map["run-1"] = db_run_id
        audit.log_step_failed("run-1", "agent-1", "step-1", "Something broke")
        mock_db.log_operation.assert_called_once()
        op = mock_db.log_operation.call_args[0][0]
        assert op.operation == "step_failed"
        assert op.status == "error"
        assert op.error == "Something broke"

    @pytest.mark.asyncio
    async def test_complete_run_calls_update(self):
        audit, mock_db = _make_enabled_audit()
        db_run_id = uuid4()
        audit._run_map["run-1"] = db_run_id
        await audit.complete_run("run-1", 500, 3000, "final output", 3, 1)
        mock_db.update_run.assert_called_once()
        call_kwargs = mock_db.update_run.call_args
        assert call_kwargs[1]["status"] == "complete"
        assert call_kwargs[1]["total_tokens"] == 500

    @pytest.mark.asyncio
    async def test_complete_run_no_mapping_noop(self):
        audit, mock_db = _make_enabled_audit()
        await audit.complete_run("unmapped", 100, 1000, "out", 1, 0)
        mock_db.update_run.assert_not_called()

    @pytest.mark.asyncio
    async def test_complete_run_error_handled(self):
        audit, mock_db = _make_enabled_audit()
        audit._run_map["run-1"] = uuid4()
        mock_db.update_run = AsyncMock(side_effect=Exception("DB error"))
        # Should not raise
        await audit.complete_run("run-1", 100, 1000, "out", 1, 0)

    @pytest.mark.asyncio
    async def test_fail_run_calls_update(self):
        audit, mock_db = _make_enabled_audit()
        db_run_id = uuid4()
        audit._run_map["run-1"] = db_run_id
        await audit.fail_run("run-1", "catastrophic failure")
        mock_db.update_run.assert_called_once()
        call_kwargs = mock_db.update_run.call_args
        assert call_kwargs[1]["status"] == "failed"
        assert call_kwargs[1]["error"] == "catastrophic failure"

    @pytest.mark.asyncio
    async def test_fail_run_error_handled(self):
        audit, mock_db = _make_enabled_audit()
        audit._run_map["run-1"] = uuid4()
        mock_db.update_run = AsyncMock(side_effect=Exception("DB error"))
        await audit.fail_run("run-1", "error")

    @pytest.mark.asyncio
    async def test_save_shared_memory_calls_db(self):
        audit, mock_db = _make_enabled_audit()
        db_run_id = uuid4()
        audit._run_map["run-1"] = db_run_id
        await audit.save_shared_memory("run-1", "execute", {"key": "value"})
        mock_db.save_shared_memory.assert_called_once_with(db_run_id, "execute", {"key": "value"})

    @pytest.mark.asyncio
    async def test_save_shared_memory_error_handled(self):
        audit, mock_db = _make_enabled_audit()
        audit._run_map["run-1"] = uuid4()
        mock_db.save_shared_memory = AsyncMock(side_effect=Exception("DB error"))
        await audit.save_shared_memory("run-1", "phase", {})

    @pytest.mark.asyncio
    async def test_get_run_operations_calls_db(self):
        audit, mock_db = _make_enabled_audit()
        db_run_id = uuid4()
        audit._run_map["run-1"] = db_run_id
        mock_db.list_operations = AsyncMock(return_value=[{"op": "test"}])
        result = await audit.get_run_operations("run-1")
        assert result == [{"op": "test"}]

    @pytest.mark.asyncio
    async def test_get_run_operations_no_mapping(self):
        audit, mock_db = _make_enabled_audit()
        result = await audit.get_run_operations("unmapped")
        assert result == []

    @pytest.mark.asyncio
    async def test_get_run_operations_error_handled(self):
        audit, mock_db = _make_enabled_audit()
        audit._run_map["run-1"] = uuid4()
        mock_db.list_operations = AsyncMock(side_effect=Exception("DB error"))
        result = await audit.get_run_operations("run-1")
        assert result == []
