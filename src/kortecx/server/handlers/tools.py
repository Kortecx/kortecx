from functools import wraps
from typing import Callable
from mcp.server.fastmcp import FastMCP
from kortecx.utils.wrappers import WrapperClass
from kortecx.thalamus.utils.catalog_utils import setupDuckDb
from typing import Any

mcp = FastMCP("Kortecx")

def register_tool(tool_name: str, tool_desc: str,  group: str = 'kxSchema', DB: str = "kortecxdb",traverse: str = "no") -> Callable[[Callable[..., Any]], Callable[..., Any]]:
    def decorator(func: Callable[..., Any]) -> Callable[..., Any]:
        duckconn = setupDuckDb(defaultDatabase=DB, func=func)
        @wraps(func)
        @mcp.tool(name=tool_name, description=tool_desc)
        def wrapper(*args: Any, **kwargs: Any) -> Any:
            project_configs = WrapperClass().get_config()
            if project_configs['UCEnabled'] != True:
                duckconn.log_tools(tool_name=tool_name, tool_desc=tool_desc, group=group, params = [*args, *kwargs])
            result = func(*args, **kwargs)
            return result
        return wrapper
    return decorator