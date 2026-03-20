"""MLflow tracking router — log and query experiments."""

from __future__ import annotations

from typing import Any

from fastapi import APIRouter
from pydantic import BaseModel

from engine.services.mlflow_tracker import mlflow_tracker

router = APIRouter()


class LogDatasetRequest(BaseModel):
    name: str
    path: str
    format: str = "jsonl"
    sampleCount: int = 0
    project: str = "default"
    tags: dict[str, str] | None = None
    schema: list[dict] | None = None


class LogChartRequest(BaseModel):
    name: str
    svgContent: str
    config: dict[str, Any] = {}
    datasetId: str = ""
    project: str = "default"


class LogModelRequest(BaseModel):
    name: str
    path: str
    framework: str = "pytorch"
    metrics: dict[str, float] | None = None
    params: dict[str, Any] | None = None
    project: str = "default"


class LogAssetRequest(BaseModel):
    name: str
    path: str
    assetType: str = "file"
    project: str = "default"
    tags: dict[str, str] | None = None


@router.get("/status")
async def mlflow_status() -> dict[str, Any]:
    """Check MLflow tracking status."""
    return mlflow_tracker.get_status()


@router.post("/log/dataset")
async def log_dataset(body: LogDatasetRequest) -> dict[str, Any]:
    """Log a dataset to MLflow."""
    run_id = mlflow_tracker.log_dataset(
        name=body.name,
        path=body.path,
        format=body.format,
        sample_count=body.sampleCount,
        project=body.project,
        tags=body.tags,
        schema=body.schema,
    )
    return {"logged": run_id is not None, "runId": run_id}


@router.post("/log/chart")
async def log_chart(body: LogChartRequest) -> dict[str, Any]:
    """Log a chart to MLflow."""
    run_id = mlflow_tracker.log_chart(
        name=body.name,
        svg_content=body.svgContent,
        config=body.config,
        dataset_id=body.datasetId,
        project=body.project,
    )
    return {"logged": run_id is not None, "runId": run_id}


@router.post("/log/model")
async def log_model(body: LogModelRequest) -> dict[str, Any]:
    """Log a model to MLflow."""
    run_id = mlflow_tracker.log_model(
        name=body.name,
        path=body.path,
        framework=body.framework,
        metrics=body.metrics,
        params=body.params,
        project=body.project,
    )
    return {"logged": run_id is not None, "runId": run_id}


@router.post("/log/asset")
async def log_asset(body: LogAssetRequest) -> dict[str, Any]:
    """Log an asset to MLflow."""
    run_id = mlflow_tracker.log_asset(
        name=body.name,
        path=body.path,
        asset_type=body.assetType,
        project=body.project,
        tags=body.tags,
    )
    return {"logged": run_id is not None, "runId": run_id}
