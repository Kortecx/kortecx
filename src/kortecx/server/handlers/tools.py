from mcp.server.fastmcp import FastMCP
from functools import wraps
import duckdb
import numpy as np
import os
import inspect

mcp = FastMCP("Kortecx")


def register_tool(tool_name: str, tool_desc: str,  group: str = 'kxSchema', DB: str = "kortecxdb",traverse: str = "no"):
    def decorator(func):
        @wraps(func)
        @mcp.tool(name=tool_name, description=tool_desc)
        def wrapper(*args, **kwargs):
            duckconn = duckdb.connect(database=DB)
            caller_file = inspect.getfile(func)
            caller_dir = os.path.join(os.path.dirname(os.path.abspath(caller_file)), caller_file)
            create_registry = f"""
                                CREATE SEQUENCE IF NOT EXISTS tools_id_seq; \n
                                CREATE TABLE IF NOT EXISTS {DB} (idx INTEGER PRIMARY KEY DEFAULT nextval('tools_id_seq'), tool_name VARCHAR, input_args VARCHAR , tool_location VARCHAR, tool_description VARCHAR,traverse VARCHAR);"""
            existing_table = duckconn.execute("SHOW TABLES").fetch_df().get("name").values
            if not np.array_equal(existing_table, DB) or not existing_table:
                duckconn.execute(create_registry)
            insert_metadata = f"INSERT INTO kortecxdb (tool_name, input_args, tool_location, tool_description, traverse) values ('{tool_name}', '{[*args,*kwargs]}','{caller_dir}','{tool_desc}','{traverse}');"
            existing_tools = duckconn.execute(f"SELECT * FROM {DB}").fetch_df().get("tool_name").values
            if tool_name not in existing_tools:
                duckconn.execute(insert_metadata)
            select_stmt = f"SELECT * FROM kortecxdb"
            print(duckconn.execute(select_stmt).fetch_df()) 
            result = func(*args, **kwargs)

            return result
        return wrapper
    return decorator