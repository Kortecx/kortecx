"""Workflow logging API — persist and retrieve interaction logs."""

from __future__ import annotations

from typing import Any

from fastapi import APIRouter
from pydantic import BaseModel

from engine.services.workflow_logger import workflow_logger

router = APIRouter()


# ── Request Models ───────────────────────────────────────────────────────────


class LogInteractionRequest(BaseModel):
    workflowId: str
    action: str
    details: dict[str, Any] | None = None


class SaveGoalRequest(BaseModel):
    workflowId: str
    goalContent: str
    source: str = "text"  # text | file


class SaveConfigRequest(BaseModel):
    workflowId: str
    config: dict[str, Any]


class MetricsConfigRequest(BaseModel):
    workflowId: str
    metricsConfig: dict[str, Any]


class SaveTagsRequest(BaseModel):
    workflowId: str
    tags: list[str]


class SavePermissionsRequest(BaseModel):
    workflowId: str
    permissions: dict[str, Any]


class SessionEventRequest(BaseModel):
    sessionId: str
    eventType: str
    data: dict[str, Any] | None = None


class StepChangeRequest(BaseModel):
    workflowId: str
    action: str  # added | removed | updated | reordered
    stepData: dict[str, Any]


# ── Routes ───────────────────────────────────────────────────────────────────


@router.post("/interaction")
async def log_interaction(req: LogInteractionRequest) -> dict[str, str]:
    """Log a user interaction event."""
    workflow_logger.log_interaction(req.workflowId, req.action, req.details)
    return {"status": "logged"}


@router.post("/goal")
async def save_goal(req: SaveGoalRequest) -> dict[str, str]:
    """Persist the task goal content."""
    workflow_logger.save_goal(req.workflowId, req.goalContent, req.source)
    return {"status": "saved"}


@router.post("/config")
async def save_config(req: SaveConfigRequest) -> dict[str, str]:
    """Persist workflow configuration snapshot."""
    workflow_logger.save_config(req.workflowId, req.config)
    return {"status": "saved"}


@router.post("/metrics")
async def save_metrics_config(req: MetricsConfigRequest) -> dict[str, str]:
    """Log metrics configuration changes."""
    workflow_logger.log_metrics_config(req.workflowId, req.metricsConfig)
    return {"status": "logged"}


@router.post("/tags")
async def save_tags(req: SaveTagsRequest) -> dict[str, str]:
    """Persist workflow tags."""
    workflow_logger.save_tags(req.workflowId, req.tags)
    return {"status": "saved"}


@router.post("/permissions")
async def save_permissions(req: SavePermissionsRequest) -> dict[str, str]:
    """Persist workflow permissions."""
    workflow_logger.save_permissions(req.workflowId, req.permissions)
    return {"status": "saved"}


@router.post("/session")
async def log_session_event(req: SessionEventRequest) -> dict[str, str]:
    """Log a session-level event."""
    workflow_logger.log_session_event(req.sessionId, req.eventType, req.data)
    return {"status": "logged"}


@router.post("/step")
async def log_step_change(req: StepChangeRequest) -> dict[str, str]:
    """Log a step add/remove/update event."""
    workflow_logger.log_step_change(req.workflowId, req.action, req.stepData)
    return {"status": "logged"}


# ── Retrieval ────────────────────────────────────────────────────────────────


@router.get("/workflow/{workflow_id}")
async def get_workflow_logs(workflow_id: str) -> dict[str, Any]:
    """List all log files for a workflow."""
    return workflow_logger.list_workflow_logs(workflow_id)


@router.get("/workflow/{workflow_id}/interactions")
async def get_interaction_log(workflow_id: str) -> dict[str, str]:
    """Get the interaction log for a workflow."""
    return {"workflowId": workflow_id, "log": workflow_logger.get_interaction_log(workflow_id)}


@router.get("/workflow/{workflow_id}/metrics")
async def get_metrics_log(workflow_id: str) -> dict[str, str]:
    """Get the metrics log for a workflow."""
    return {"workflowId": workflow_id, "log": workflow_logger.get_metrics_log(workflow_id)}


@router.get("/run/{workflow_id}/{run_id}")
async def get_run_log(workflow_id: str, run_id: str) -> dict[str, str]:
    """Get execution log for a specific run."""
    return {"workflowId": workflow_id, "runId": run_id, "log": workflow_logger.get_run_log(workflow_id, run_id)}


@router.get("/session/{session_id}")
async def get_session_log(session_id: str) -> dict[str, str]:
    """Get a session log."""
    return {"sessionId": session_id, "log": workflow_logger.get_session_log(session_id)}
