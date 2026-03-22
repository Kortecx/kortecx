from __future__ import annotations

from pydantic_settings import BaseSettings


class Settings(BaseSettings):
    """Engine configuration — reads from environment / .env file."""

    # Server
    host: str = "0.0.0.0"
    port: int = 8000
    debug: bool = False

    # Database (PostgreSQL)
    database_url: str = "postgresql://kortecx:kortecx@localhost:5433/kortecx_dev"

    # DuckDB
    duckdb_path: str = ":memory:"

    # Qdrant
    qdrant_url: str = "http://localhost:6333"
    qdrant_collection: str = "kortecx_embeddings"

    # HuggingFace
    hf_token: str = ""

    # Spark
    spark_master: str = "local[*]"
    spark_app_name: str = "kortecx-engine"

    # Local inference
    ollama_url: str = "http://localhost:11434"
    llamacpp_url: str = "http://localhost:8080"

    # Orchestration
    upload_dir: str = "./uploads"
    max_concurrent_agents: int = 10
    agent_retry_enabled: bool = True
    agent_fallback_model: str = "llama3.2:3b"
    default_local_engine: str = "ollama"
    default_local_model: str = "llama3.1:8b"

    # Quorum
    quorum_max_concurrent: int = 4
    quorum_metrics_interval: float = 5.0
    quorum_default_workers: int = 3
    quorum_default_retries: int = 3

    model_config = {"env_file": "../.env", "env_file_encoding": "utf-8", "extra": "ignore"}


settings = Settings()
