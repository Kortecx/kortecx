from __future__ import annotations

import logging
from typing import Any

import duckdb

from engine.config import settings

logger = logging.getLogger("engine.duckdb")


class DuckDBService:
    """Analytical query engine backed by DuckDB."""

    def __init__(self) -> None:
        self._conn: duckdb.DuckDBPyConnection | None = None

    @property
    def conn(self) -> duckdb.DuckDBPyConnection:
        if self._conn is None:
            self._conn = duckdb.connect(settings.duckdb_path)
            logger.info("DuckDB connected: %s", settings.duckdb_path)
        return self._conn

    def ping(self) -> bool:
        self.conn.execute("SELECT 1")
        return True

    def execute(self, query: str, params: list[Any] | None = None) -> list[dict[str, Any]]:
        """Run a SQL query and return rows as dicts."""
        rel = self.conn.execute(query, params or [])
        cols = [desc[0] for desc in rel.description]
        return [dict(zip(cols, row)) for row in rel.fetchall()]

    def register_dataframe(self, name: str, df: Any) -> None:
        """Register a pandas/polars DataFrame as a virtual table."""
        self.conn.register(name, df)

    def load_parquet(self, table_name: str, path: str) -> int:
        """Load a Parquet file into a DuckDB table, return row count."""
        self.conn.execute(f"CREATE OR REPLACE TABLE {table_name} AS SELECT * FROM read_parquet(?)", [path])
        result = self.conn.execute(f"SELECT count(*) FROM {table_name}").fetchone()
        return result[0] if result else 0

    def load_csv(self, table_name: str, path: str) -> int:
        """Load a CSV file into a DuckDB table, return row count."""
        self.conn.execute(f"CREATE OR REPLACE TABLE {table_name} AS SELECT * FROM read_csv_auto(?)", [path])
        result = self.conn.execute(f"SELECT count(*) FROM {table_name}").fetchone()
        return result[0] if result else 0

    def query_table(self, table_name: str, limit: int = 100, offset: int = 0) -> list[dict[str, Any]]:
        return self.execute(f"SELECT * FROM {table_name} LIMIT ? OFFSET ?", [limit, offset])

    def list_tables(self) -> list[str]:
        rows = self.execute("SELECT table_name FROM information_schema.tables WHERE table_schema = 'main'")
        return [r["table_name"] for r in rows]


duckdb_service = DuckDBService()
