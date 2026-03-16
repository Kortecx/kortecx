from __future__ import annotations

from typing import Any

from fastapi import APIRouter, HTTPException, Query
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
    import os

    import duckdb

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
        cols_result = conn.execute("SELECT column_name, data_type FROM information_schema.columns WHERE table_name = 'data_view'").fetchall()
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
                string_cols = [c["name"] for c in columns if "VARCHAR" in c["type"].upper() or "TEXT" in c["type"].upper()]
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

        # Use fetchall + column names to avoid numpy serialization issues
        result_set = conn.execute(sql)
        raw_rows = result_set.fetchall()
        col_names = [desc[0] for desc in result_set.description] if result_set.description else column_names
        rows = []
        for raw in raw_rows:
            row: dict[str, Any] = {}
            for j, val in enumerate(raw):
                # Convert numpy/non-serializable types to Python natives
                if val is None:
                    row[col_names[j]] = None
                elif hasattr(val, "item"):  # numpy scalar
                    row[col_names[j]] = val.item()
                elif isinstance(val, (list, dict)):
                    row[col_names[j]] = val
                else:
                    row[col_names[j]] = val
            rows.append(row)

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
                filtered_rows = conn.execute(f"SELECT COUNT(*) FROM data_view WHERE {' AND '.join(where_parts_count)}").fetchone()[0]

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


# ---------------------------------------------------------------------------
# Schema detection
# ---------------------------------------------------------------------------


class SchemaDetectRequest(BaseModel):
    path: str


@router.post("/file/schema")
async def detect_schema(req: SchemaDetectRequest) -> dict[str, Any]:
    """Detect schema (column names + types) from a CSV/JSONL/Delta file."""
    import asyncio

    return await asyncio.to_thread(_detect_schema_sync, req.path)


def _detect_schema_sync(path: str) -> dict[str, Any]:
    import os

    import duckdb

    if not os.path.exists(path):
        return {"error": f"File not found: {path}", "columns": []}
    conn = duckdb.connect()
    try:
        lower = path.lower()
        if lower.endswith(".csv"):
            conn.execute(f"CREATE VIEW data_view AS SELECT * FROM read_csv_auto('{path}')")
        elif lower.endswith(".jsonl") or lower.endswith(".json"):
            conn.execute(f"CREATE VIEW data_view AS SELECT * FROM read_json_auto('{path}')")
        elif ".delta." in lower or lower.endswith(".parquet"):
            conn.execute(f"CREATE VIEW data_view AS SELECT * FROM read_json_auto('{path}')")
        else:
            conn.execute(f"CREATE VIEW data_view AS SELECT * FROM read_json_auto('{path}')")

        cols = conn.execute("SELECT column_name, data_type FROM information_schema.columns WHERE table_name = 'data_view'").fetchall()
        total = conn.execute("SELECT COUNT(*) FROM data_view").fetchone()[0]

        columns: list[dict[str, Any]] = []
        for name, dtype in cols:
            # Map DuckDB types to simpler types
            simple = "string"
            upper = dtype.upper()
            if "INT" in upper or "BIGINT" in upper or "SMALLINT" in upper or "TINYINT" in upper:
                simple = "integer"
            elif "DOUBLE" in upper or "FLOAT" in upper or "DECIMAL" in upper or "NUMERIC" in upper:
                simple = "float"
            elif "BOOL" in upper:
                simple = "boolean"
            elif "TIMESTAMP" in upper or "DATETIME" in upper:
                simple = "timestamp"
            elif "DATE" in upper:
                simple = "date"
            elif "JSON" in upper or "STRUCT" in upper or "MAP" in upper:
                simple = "json"
            elif "ARRAY" in upper or "LIST" in upper:
                simple = "array"
            elif "BLOB" in upper:
                simple = "binary"
            columns.append(
                {
                    "name": name,
                    "type": simple,
                    "duckdbType": dtype,
                    "description": "",
                    "required": False,
                }
            )

        return {"columns": columns, "totalRows": total, "path": path}
    except Exception as exc:
        return {"error": str(exc), "columns": []}
    finally:
        conn.close()


# ---------------------------------------------------------------------------
# File update (batch edit rows)
# ---------------------------------------------------------------------------


class FileUpdateRequest(BaseModel):
    path: str
    updates: list[dict[str, Any]]  # [{rowIndex: int, column: str, value: Any}, ...]
    create_version: bool = True  # save a version snapshot before editing


@router.post("/file/update")
async def update_file(req: FileUpdateRequest) -> dict[str, Any]:
    """Update rows in a CSV/JSONL/Delta file. Creates a version snapshot first."""
    import asyncio

    return await asyncio.to_thread(_update_file_sync, req)


def _update_file_sync(req: FileUpdateRequest) -> dict[str, Any]:
    import os
    import shutil
    from datetime import datetime

    path = req.path
    if not os.path.exists(path):
        return {"error": f"File not found: {path}", "updated": 0}

    lower = path.lower()

    # Create version snapshot
    version_path: str | None = None
    if req.create_version:
        version_dir = os.path.join(os.path.dirname(path), ".versions")
        os.makedirs(version_dir, exist_ok=True)
        ts = datetime.utcnow().strftime("%Y%m%d_%H%M%S")
        base = os.path.basename(path)
        version_path = os.path.join(version_dir, f"{base}.v{ts}")
        shutil.copy2(path, version_path)

    try:
        if lower.endswith(".jsonl") or lower.endswith(".json") or ".delta." in lower:
            return _update_jsonl(path, req.updates, version_path)
        elif lower.endswith(".csv"):
            return _update_csv(path, req.updates, version_path)
        else:
            return {"error": f"Unsupported format for editing: {path}", "updated": 0}
    except Exception as exc:
        return {"error": str(exc), "updated": 0}


def _update_jsonl(path: str, updates: list[dict[str, Any]], version_path: str | None) -> dict[str, Any]:
    import json

    # Read all rows
    with open(path, encoding="utf-8") as f:
        lines = f.readlines()

    rows: list[dict[str, Any]] = []
    for line in lines:
        line = line.strip()
        if line:
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                rows.append({"_raw": line})

    # Apply updates
    updated = 0
    for upd in updates:
        idx = upd.get("rowIndex")
        col = upd.get("column")
        val = upd.get("value")
        if idx is not None and 0 <= idx < len(rows) and col:
            rows[idx][col] = val
            updated += 1

    # Write back
    with open(path, "w", encoding="utf-8") as f:
        for row in rows:
            f.write(json.dumps(row, ensure_ascii=False) + "\n")

    return {"updated": updated, "totalRows": len(rows), "versionPath": version_path}


def _update_csv(path: str, updates: list[dict[str, Any]], version_path: str | None) -> dict[str, Any]:
    import csv

    # Read all rows
    with open(path, encoding="utf-8", newline="") as f:
        reader = csv.DictReader(f)
        fieldnames = reader.fieldnames or []
        rows = list(reader)

    # Apply updates
    updated = 0
    for upd in updates:
        idx = upd.get("rowIndex")
        col = upd.get("column")
        val = upd.get("value")
        if idx is not None and 0 <= idx < len(rows) and col and col in fieldnames:
            rows[idx][col] = val
            updated += 1

    # Write back
    with open(path, "w", encoding="utf-8", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(rows)

    return {"updated": updated, "totalRows": len(rows), "versionPath": version_path}


# ---------------------------------------------------------------------------
# Version history
# ---------------------------------------------------------------------------


class VersionListRequest(BaseModel):
    path: str


@router.post("/file/versions")
async def list_versions(req: VersionListRequest) -> dict[str, Any]:
    """List version snapshots for a data file."""
    import os
    from datetime import datetime

    version_dir = os.path.join(os.path.dirname(req.path), ".versions")
    base = os.path.basename(req.path)

    if not os.path.exists(version_dir):
        return {"versions": [], "path": req.path}

    versions: list[dict[str, Any]] = []
    for f in sorted(os.listdir(version_dir), reverse=True):
        if f.startswith(base + ".v"):
            full = os.path.join(version_dir, f)
            stat = os.stat(full)
            ts_part = f.split(".v")[-1]
            versions.append(
                {
                    "name": f,
                    "path": full,
                    "sizeBytes": stat.st_size,
                    "timestamp": ts_part,
                    "createdAt": datetime.fromtimestamp(stat.st_mtime).isoformat(),
                }
            )

    return {"versions": versions, "path": req.path, "count": len(versions)}


class VersionRestoreRequest(BaseModel):
    original_path: str
    version_path: str


@router.post("/file/restore")
async def restore_version(req: VersionRestoreRequest) -> dict[str, Any]:
    """Restore a file from a version snapshot (creates a new version of current first)."""
    import asyncio

    return await asyncio.to_thread(_restore_version_sync, req)


def _restore_version_sync(req: VersionRestoreRequest) -> dict[str, Any]:
    import os
    import shutil
    from datetime import datetime

    if not os.path.exists(req.version_path):
        return {"error": "Version file not found"}

    # Save current as a version first
    version_dir = os.path.join(os.path.dirname(req.original_path), ".versions")
    os.makedirs(version_dir, exist_ok=True)
    ts = datetime.utcnow().strftime("%Y%m%d_%H%M%S")
    base = os.path.basename(req.original_path)
    shutil.copy2(req.original_path, os.path.join(version_dir, f"{base}.v{ts}_pre_restore"))

    # Restore
    shutil.copy2(req.version_path, req.original_path)

    return {"restored": True, "from": req.version_path}


# ---------------------------------------------------------------------------
# Schema alter (rename / retype / add / drop columns)
# ---------------------------------------------------------------------------


class SchemaAlterRequest(BaseModel):
    path: str
    renames: dict[str, str] | None = None  # {oldName: newName}
    type_changes: dict[str, str] | None = None  # {colName: newType} — applied as cast
    drop_columns: list[str] | None = None
    add_columns: list[dict[str, Any]] | None = None  # [{name, type, default}]
    create_version: bool = True


@router.post("/file/alter-schema")
async def alter_schema(req: SchemaAlterRequest) -> dict[str, Any]:
    """Alter schema of a CSV/JSONL file — rename, retype, add, drop columns."""
    import asyncio

    return await asyncio.to_thread(_alter_schema_sync, req)


def _alter_schema_sync(req: SchemaAlterRequest) -> dict[str, Any]:
    import os
    import shutil
    from datetime import datetime

    import duckdb

    path = req.path
    if not os.path.exists(path):
        return {"error": f"File not found: {path}"}

    # Version snapshot
    if req.create_version:
        version_dir = os.path.join(os.path.dirname(path), ".versions")
        os.makedirs(version_dir, exist_ok=True)
        ts = datetime.utcnow().strftime("%Y%m%d_%H%M%S")
        base = os.path.basename(path)
        shutil.copy2(path, os.path.join(version_dir, f"{base}.v{ts}"))

    lower = path.lower()
    conn = duckdb.connect()

    try:
        # Read into DuckDB
        if lower.endswith(".csv"):
            conn.execute(f"CREATE TABLE data_tbl AS SELECT * FROM read_csv_auto('{path}')")
        else:
            conn.execute(f"CREATE TABLE data_tbl AS SELECT * FROM read_json_auto('{path}')")

        # Drop columns
        if req.drop_columns:
            for col in req.drop_columns:
                try:
                    conn.execute(f'ALTER TABLE data_tbl DROP COLUMN "{col}"')
                except Exception:
                    pass

        # Rename columns
        if req.renames:
            for old_name, new_name in req.renames.items():
                try:
                    conn.execute(f'ALTER TABLE data_tbl RENAME COLUMN "{old_name}" TO "{new_name}"')
                except Exception:
                    pass

        # Add columns
        if req.add_columns:
            for col_def in req.add_columns:
                name = col_def.get("name", "")
                dtype = col_def.get("type", "VARCHAR")
                default = col_def.get("default", "NULL")
                if name:
                    try:
                        conn.execute(f'ALTER TABLE data_tbl ADD COLUMN "{name}" {dtype} DEFAULT {default}')
                    except Exception:
                        pass

        # Write back
        tmp_path = path + ".tmp"
        if lower.endswith(".csv"):
            conn.execute(f"COPY data_tbl TO '{tmp_path}' (HEADER, DELIMITER ',')")
        else:
            conn.execute(f"COPY data_tbl TO '{tmp_path}' (FORMAT JSON, ARRAY true)")
            # Convert JSON array to JSONL
            import json

            with open(tmp_path) as f:
                arr = json.load(f)
            with open(tmp_path, "w") as f:
                for row in arr:
                    f.write(json.dumps(row, ensure_ascii=False) + "\n")

        os.replace(tmp_path, path)

        # Get new schema
        return _detect_schema_sync(path)

    except Exception as exc:
        return {"error": str(exc)}
    finally:
        conn.close()
