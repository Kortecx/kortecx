from kortecx.server.handlers.tools import register_tool

@register_tool("Addition","functions that helps add two numbers")
def add(x,y):
    return x+y

if __name__ == '__main__':
    add(1,2)