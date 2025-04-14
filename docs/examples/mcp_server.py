from kortecx.server.handlers.tools import register_tool
import duckdb

@register_tool("Addition","functions that helps add two numbers")
def add_numbers(x,y):
    return x+y

if __name__ == '__main__':
    add_numbers(1,2)