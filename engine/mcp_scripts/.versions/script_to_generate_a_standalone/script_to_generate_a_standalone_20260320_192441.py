import os
import json
from mcp.server import Server

class MCPServer(Server):
    def __init__(self):
        super().__init__()
        self.name = "MCP React Generator"
        self.description = "Generates a standalone webpage in React"

    def on_message(self, message):
        if message['type'] == 'generate':
            project_name = message['data']['projectName']
            output_dir = f"./{project_name}"

            # Create the directory for the project
            os.makedirs(output_dir, exist_ok=True)

            # Generate the basic React app structure
            with open(f"{output_dir}/index.html", "w") as f:
                f.write("<!DOCTYPE html><html><head><title>{project_name}</title></head><body></body></html>")

            with open(f"{output_dir}/src/App.js", "w") as f:
                f.write("import React from 'react';\n")
                f.write("function App() {\n")
                f.write("  return <div>Hello World!</div>;\n")
                f.write("}\n")
                f.write("export default App;")

            with open(f"{output_dir}/package.json", "w") as f:
                json.dump({
                    "name": project_name,
                    "version": "1.0.0",
                    "scripts": {
                        "start": "react-scripts start"
                    },
                    "dependencies": {
                        "react": "^17.0.2",
                        "react-dom": "^17.0.2",
                        "react-scripts": "^4.0.3"
                    }
                }, f)

            return {"type": "success", "data": {"message": f"Project generated in {output_dir}"}}
        else:
            return {"type": "error", "data": {"message": "Invalid request"}}

if __name__ == "__main__":
    server = MCPServer()
    server.listen(8080)
    print("MCP Server listening on port 8080")