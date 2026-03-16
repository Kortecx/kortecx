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


@router.get("/table/{table_name}")
async def get_table_data(
    table_name: str,
    limit: int = Query(100, ge=1, le=10000),
    offset: int = Query(0, ge=0),
):
    """Preview rows from a table."""
    rows = duckdb_service.query_table(table_name, limit=limit, offset=offset)
    return {"table": table_name, "rows": rows, "count": len(rows)}
