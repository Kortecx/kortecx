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

    model_config = {"env_file": "../.env", "env_file_encoding": "utf-8", "extra": "ignore"}


settings = Settings()
