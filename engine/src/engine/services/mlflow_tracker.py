"""MLflow tracking service — logs experiments, artifacts, metrics, and models."""

from __future__ import annotations

import logging
import os
from typing import Any

logger = logging.getLogger("engine.mlflow")

# MLflow is optional — gracefully degrade if not available
try:
    import mlflow
    from mlflow.tracking import MlflowClient

    HAS_MLFLOW = True
except ImportError:
    HAS_MLFLOW = False
    logger.info("MLflow not installed — tracking disabled")


class MLflowTracker:
    """Wraps MLflow tracking for datasets, models, charts, and training runs."""

    def __init__(self) -> None:
        self._tracking_uri = os.getenv("MLFLOW_TRACKING_URI", "http://localhost:5050")
        self._enabled = HAS_MLFLOW and os.getenv("MLFLOW_ENABLED", "true").lower() == "true"
        self._client: MlflowClient | None = None
        if self._enabled:
            try:
                mlflow.set_tracking_uri(self._tracking_uri)
                self._client = MlflowClient(self._tracking_uri)
                logger.info("MLflow tracking enabled at %s", self._tracking_uri)
            except Exception as exc:
                logger.warning("MLflow connection failed: %s — tracking disabled", exc)
                self._enabled = False

    @property
    def enabled(self) -> bool:
        return self._enabled

    def _ensure_experiment(self, name: str) -> str:
        """Get or create an experiment, return its ID."""
        if not self._client:
            return "0"
        exp = self._client.get_experiment_by_name(name)
        if exp:
            return exp.experiment_id
        return self._client.create_experiment(name)

    def log_dataset(
        self,
        *,
        name: str,
        path: str,
        format: str,
        sample_count: int,
        project: str = "default",
        tags: dict[str, str] | None = None,
        schema: list[dict] | None = None,
    ) -> str | None:
        """Log a dataset as an MLflow run with artifact."""
        if not self._enabled:
            return None
        try:
            exp_id = self._ensure_experiment(f"kortecx-{project}-datasets")
            with mlflow.start_run(experiment_id=exp_id, run_name=f"dataset-{name}") as run:
                mlflow.log_param("name", name)
                mlflow.log_param("format", format)
                mlflow.log_param("project", project)
                mlflow.log_metric("sample_count", sample_count)
                if tags:
                    mlflow.set_tags(tags)
                if schema:
                    mlflow.log_dict(schema, "schema.json")
                if os.path.exists(path):
                    mlflow.log_artifact(path, artifact_path="datasets")
                return run.info.run_id
        except Exception as exc:
            logger.warning("MLflow dataset log failed: %s", exc)
            return None

    def log_chart(
        self,
        *,
        name: str,
        svg_content: str,
        config: dict[str, Any],
        dataset_id: str = "",
        project: str = "default",
    ) -> str | None:
        """Log a chart as an MLflow artifact."""
        if not self._enabled:
            return None
        try:
            exp_id = self._ensure_experiment(f"kortecx-{project}-charts")
            with mlflow.start_run(experiment_id=exp_id, run_name=f"chart-{name}") as run:
                mlflow.log_param("chart_type", config.get("chartType", ""))
                mlflow.log_param("dataset_id", dataset_id)
                mlflow.log_param("project", project)
                mlflow.log_dict(config, "chart_config.json")
                # Save SVG as artifact
                import tempfile

                with tempfile.NamedTemporaryFile(suffix=".svg", mode="w", delete=False) as f:
                    f.write(svg_content)
                    tmp_path = f.name
                mlflow.log_artifact(tmp_path, artifact_path="charts")
                os.unlink(tmp_path)
                return run.info.run_id
        except Exception as exc:
            logger.warning("MLflow chart log failed: %s", exc)
            return None

    def log_model(
        self,
        *,
        name: str,
        path: str,
        framework: str = "pytorch",
        metrics: dict[str, float] | None = None,
        params: dict[str, Any] | None = None,
        project: str = "default",
    ) -> str | None:
        """Log a trained model as an MLflow run."""
        if not self._enabled:
            return None
        try:
            exp_id = self._ensure_experiment(f"kortecx-{project}-models")
            with mlflow.start_run(experiment_id=exp_id, run_name=f"model-{name}") as run:
                mlflow.log_param("model_name", name)
                mlflow.log_param("framework", framework)
                mlflow.log_param("project", project)
                if params:
                    mlflow.log_params({k: str(v)[:250] for k, v in params.items()})
                if metrics:
                    mlflow.log_metrics(metrics)
                if os.path.isdir(path):
                    mlflow.log_artifacts(path, artifact_path="model")
                elif os.path.exists(path):
                    mlflow.log_artifact(path, artifact_path="model")
                return run.info.run_id
        except Exception as exc:
            logger.warning("MLflow model log failed: %s", exc)
            return None

    def log_training_run(
        self,
        *,
        job_id: str,
        name: str,
        dataset_id: str,
        base_model: str,
        method: str,
        epochs: int,
        learning_rate: float,
        metrics: dict[str, float] | None = None,
        output_dir: str = "",
        project: str = "default",
    ) -> str | None:
        """Log a training run with hyperparams and metrics."""
        if not self._enabled:
            return None
        try:
            exp_id = self._ensure_experiment(f"kortecx-{project}-training")
            with mlflow.start_run(experiment_id=exp_id, run_name=f"train-{name}") as run:
                mlflow.log_param("job_id", job_id)
                mlflow.log_param("base_model", base_model)
                mlflow.log_param("method", method)
                mlflow.log_param("dataset_id", dataset_id)
                mlflow.log_param("project", project)
                mlflow.log_param("epochs", epochs)
                mlflow.log_param("learning_rate", learning_rate)
                if metrics:
                    mlflow.log_metrics(metrics)
                if output_dir and os.path.isdir(output_dir):
                    mlflow.log_artifacts(output_dir, artifact_path="training_output")
                return run.info.run_id
        except Exception as exc:
            logger.warning("MLflow training log failed: %s", exc)
            return None

    def log_asset(
        self,
        *,
        name: str,
        path: str,
        asset_type: str = "file",
        project: str = "default",
        tags: dict[str, str] | None = None,
    ) -> str | None:
        """Log any asset (document, script, etc.) as an MLflow artifact."""
        if not self._enabled:
            return None
        try:
            exp_id = self._ensure_experiment(f"kortecx-{project}-assets")
            with mlflow.start_run(experiment_id=exp_id, run_name=f"asset-{name}") as run:
                mlflow.log_param("asset_name", name)
                mlflow.log_param("asset_type", asset_type)
                mlflow.log_param("project", project)
                if tags:
                    mlflow.set_tags(tags)
                if os.path.exists(path):
                    mlflow.log_artifact(path, artifact_path="assets")
                return run.info.run_id
        except Exception as exc:
            logger.warning("MLflow asset log failed: %s", exc)
            return None

    def get_status(self) -> dict[str, Any]:
        """Return MLflow connection status."""
        return {
            "enabled": self._enabled,
            "tracking_uri": self._tracking_uri,
            "has_mlflow": HAS_MLFLOW,
        }


mlflow_tracker = MLflowTracker()
