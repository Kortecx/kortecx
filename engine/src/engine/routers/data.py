from __future__ import annotations

from typing import Any

from fastapi import APIRouter, HTTPException, UploadFile, File, Query
from pydantic import BaseModel

from engine.services.duckdb import duckdb_service
from engine.services.spark import spark_service

router = APIRouter()


class QueryRequest(BaseModel):
    sql: str
    engine: str = "duckdb"  # "duckdb" | "spark"


class LoadRequest(BaseModel):
    table_name: str
    path: str
    format: str = "parquet"  # "parquet" | "csv" | "json"
    engine: str = "duckdb"


@router.get("/tables")
async def list_tables():
    """List all DuckDB tables."""
    return {"tables": duckdb_service.list_tables()}


@router.post("/query")
async def run_query(req: QueryRequest) -> dict[str, Any]:
    """Execute a SQL query against DuckDB or Spark."""
    if req.engine == "spark":
        df = spark_service.sql(req.sql)
        rows = [row.asDict() for row in df.collect()]
        return {"rows": rows, "count": len(rows), "engine": "spark"}

    rows = duckdb_service.execute(req.sql)
    return {"rows": rows, "count": len(rows), "engine": "duckdb"}


@router.post("/load")
async def load_data(req: LoadRequest) -> dict[str, Any]:
    """Load a file into a queryable table."""
    if req.engine == "spark":
        if req.format == "parquet":
            df = spark_service.read_parquet(req.path)
        elif req.format == "csv":
            df = spark_service.read_csv(req.path)
        elif req.format == "json":
            df = spark_service.read_json(req.path)
        else:
            raise HTTPException(400, f"Unsupported format: {req.format}")
        spark_service.register_temp_view(df, req.table_name)
        return {"table": req.table_name, "count": df.count(), "engine": "spark"}

    if req.format == "parquet":
        count = duckdb_service.load_parquet(req.table_name, req.path)
    elif req.format == "csv":
        count = duckdb_service.load_csv(req.table_name, req.path)
    else:
        raise HTTPException(400, f"Unsupported format for DuckDB: {req.format}")

    return {"table": req.table_name, "count": count, "engine": "duckdb"}


class FileQueryRequest(BaseModel):
    path: str
    sql: str | None = None  # optional custom SQL; if None, does SELECT *
    limit: int = 100
    offset: int = 0
    search: str | None = None  # full-text search across all string columns
    filters: dict[str, str] | None = None  # column -> value filters


@router.post("/file/query")
async def query_file(req: FileQueryRequest) -> dict[str, Any]:
    """Query a data file directly using DuckDB — supports JSONL, CSV, Parquet, Delta."""
    import asyncio

    result = await asyncio.to_thread(_query_file_sync, req)
    return result


def _query_file_sync(req: FileQueryRequest) -> dict[str, Any]:
    """Synchronous file query via DuckDB."""
    import duckdb
    import os

    path = req.path
    if not os.path.exists(path):
        return {"error": f"File not found: {path}", "rows": [], "columns": [], "totalRows": 0}

    conn = duckdb.connect()

    try:
        # Detect format and create a view
        lower = path.lower()
        if lower.endswith(".jsonl") or lower.endswith(".json"):
            conn.execute(f"CREATE VIEW data_view AS SELECT * FROM read_json_auto('{path}')")
        elif lower.endswith(".csv"):
            conn.execute(f"CREATE VIEW data_view AS SELECT * FROM read_csv_auto('{path}')")
        elif lower.endswith(".parquet"):
            conn.execute(f"CREATE VIEW data_view AS SELECT * FROM read_parquet('{path}')")
        elif ".delta." in lower:
            # Delta format stored as JSONL
            conn.execute(f"CREATE VIEW data_view AS SELECT * FROM read_json_auto('{path}')")
        else:
            # Try JSON auto as fallback
            conn.execute(f"CREATE VIEW data_view AS SELECT * FROM read_json_auto('{path}')")

        # Get column info
        cols_result = conn.execute(
            "SELECT column_name, data_type FROM information_schema.columns WHERE table_name = 'data_view'"
        ).fetchall()
        columns = [{"name": r[0], "type": r[1]} for r in cols_result]
        column_names = [r[0] for r in cols_result]

        # Get total row count
        total_rows = conn.execute("SELECT COUNT(*) FROM data_view").fetchone()[0]

        # Build query
        if req.sql:
            sql = req.sql
        else:
            where_parts: list[str] = []

            # Apply column filters
            if req.filters:
                for col, val in req.filters.items():
                    if col in column_names:
                        safe_val = val.replace("'", "''")
                        where_parts.append(f"CAST(\"{col}\" AS VARCHAR) ILIKE '%{safe_val}%'")

            # Apply full-text search across string columns
            if req.search:
                search_term = req.search.replace("'", "''")
                string_cols = [
                    c["name"]
                    for c in columns
                    if "VARCHAR" in c["type"].upper() or "TEXT" in c["type"].upper()
                ]
                if string_cols:
                    or_parts = [f"CAST(\"{c}\" AS VARCHAR) ILIKE '%{search_term}%'" for c in string_cols]
                    where_parts.append(f"({' OR '.join(or_parts)})")
                else:
                    # Search all columns as text
                    or_parts = [f"CAST(\"{c['name']}\" AS VARCHAR) ILIKE '%{search_term}%'" for c in columns]
                    if or_parts:
                        where_parts.append(f"({' OR '.join(or_parts)})")

            where_clause = f" WHERE {' AND '.join(where_parts)}" if where_parts else ""
            sql = f"SELECT * FROM data_view{where_clause} LIMIT {req.limit} OFFSET {req.offset}"

        rows = conn.execute(sql).fetchdf().to_dict(orient="records")

        # Get filtered count if filters applied
        filtered_rows = total_rows
        if req.filters or req.search:
            where_parts_count: list[str] = []
            if req.filters:
                for col, val in req.filters.items():
                    if col in column_names:
                        safe_val = val.replace("'", "''")
                        where_parts_count.append(f"CAST(\"{col}\" AS VARCHAR) ILIKE '%{safe_val}%'")
            if req.search:
                search_term = req.search.replace("'", "''")
                or_parts = [f"CAST(\"{c['name']}\" AS VARCHAR) ILIKE '%{search_term}%'" for c in columns]
                if or_parts:
                    where_parts_count.append(f"({' OR '.join(or_parts)})")
            if where_parts_count:
                filtered_rows = conn.execute(
                    f"SELECT COUNT(*) FROM data_view WHERE {' AND '.join(where_parts_count)}"
                ).fetchone()[0]

        return {
            "rows": rows,
            "columns": columns,
            "totalRows": total_rows,
            "filteredRows": filtered_rows,
            "limit": req.limit,
            "offset": req.offset,
        }
    except Exception as exc:
        return {"error": str(exc), "rows": [], "columns": [], "totalRows": 0}
    finally:
        conn.close()


@router.get("/table/{table_name}")
async def get_table_data(
    table_name: str,
    limit: int = Query(100, ge=1, le=10000),
    offset: int = Query(0, ge=0),
):
    """Preview rows from a table."""
    rows = duckdb_service.query_table(table_name, limit=limit, offset=offset)
    return {"table": table_name, "rows": rows, "count": len(rows)}
