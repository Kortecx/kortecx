# Generate a standalone React webpage project from a description
"""MCP server that generates a complete standalone React webpage with Vite, Tailwind, and TypeScript."""

import json
import os
from pathlib import Path


PACKAGE_JSON = {
    "name": "",
    "private": True,
    "version": "1.0.0",
    "type": "module",
    "scripts": {
        "dev": "vite",
        "build": "tsc && vite build",
        "preview": "vite preview",
    },
    "dependencies": {
        "react": "^19.0.0",
        "react-dom": "^19.0.0",
    },
    "devDependencies": {
        "@types/react": "^19",
        "@types/react-dom": "^19",
        "@vitejs/plugin-react": "^4.3.0",
        "typescript": "^5.6.0",
        "vite": "^6.0.0",
        "tailwindcss": "^4.0.0",
        "@tailwindcss/vite": "^4.0.0",
    },
}

VITE_CONFIG = '''import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
});
'''

TSCONFIG = '''{
  "compilerOptions": {
    "target": "ES2020",
    "module": "ESNext",
    "lib": ["ES2020", "DOM", "DOM.Iterable"],
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true
  },
  "include": ["src"]
}
'''

INDEX_HTML = '''<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>{title}</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
'''

MAIN_TSX = '''import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
'''

INDEX_CSS = '''@import "tailwindcss";
'''

def _make_app(title: str, description: str) -> str:
    return (
        'export default function App() {\n'
        '  return (\n'
        '    <div className="min-h-screen bg-gradient-to-br from-gray-900 to-gray-800 flex items-center justify-center">\n'
        '      <div className="text-center space-y-6">\n'
        '        <h1 className="text-5xl font-bold text-white tracking-tight">\n'
        f'          {title}\n'
        '        </h1>\n'
        '        <p className="text-lg text-gray-400 max-w-md mx-auto">\n'
        f'          {description}\n'
        '        </p>\n'
        '        <button className="px-6 py-3 bg-blue-600 hover:bg-blue-500 text-white font-semibold rounded-lg transition-colors">\n'
        '          Get Started\n'
        '        </button>\n'
        '      </div>\n'
        '    </div>\n'
        '  );\n'
        '}\n'
    )


def handle_request(request: dict) -> dict:
    """Handle an MCP tool call request."""
    method = request.get("method", "")

    if method == "tools/list":
        return {
            "tools": [
                {
                    "name": "generate_react_page",
                    "description": "Generate a complete standalone React + Vite + Tailwind project",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_name": {
                                "type": "string",
                                "description": "Project directory name (lowercase, no spaces)",
                            },
                            "title": {
                                "type": "string",
                                "description": "Page title displayed in the browser and heading",
                            },
                            "description": {
                                "type": "string",
                                "description": "Short description shown on the page",
                                "default": "Built with React, Vite, and Tailwind CSS",
                            },
                            "output_dir": {
                                "type": "string",
                                "description": "Parent directory for the project (default: current dir)",
                                "default": ".",
                            },
                        },
                        "required": ["project_name", "title"],
                    },
                }
            ]
        }

    if method == "tools/call":
        tool_name = request.get("params", {}).get("name", "")
        args = request.get("params", {}).get("arguments", {})

        if tool_name == "generate_react_page":
            project_name = args.get("project_name", "my-react-app")
            title = args.get("title", "My App")
            description = args.get("description", "Built with React, Vite, and Tailwind CSS")
            output_dir = Path(args.get("output_dir", ".")) / project_name

            try:
                # Create project structure
                (output_dir / "src").mkdir(parents=True, exist_ok=True)

                # package.json
                pkg = {**PACKAGE_JSON, "name": project_name}
                (output_dir / "package.json").write_text(json.dumps(pkg, indent=2))

                # vite.config.ts
                (output_dir / "vite.config.ts").write_text(VITE_CONFIG)

                # tsconfig.json
                (output_dir / "tsconfig.json").write_text(TSCONFIG)

                # index.html
                (output_dir / "index.html").write_text(INDEX_HTML.format(title=title))

                # src/main.tsx
                (output_dir / "src" / "main.tsx").write_text(MAIN_TSX)

                # src/index.css
                (output_dir / "src" / "index.css").write_text(INDEX_CSS)

                # src/App.tsx
                (output_dir / "src" / "App.tsx").write_text(_make_app(title, description))

                files = [
                    "package.json", "vite.config.ts", "tsconfig.json",
                    "index.html", "src/main.tsx", "src/index.css", "src/App.tsx",
                ]
                return {
                    "content": [
                        {
                            "type": "text",
                            "text": (
                                f"React project generated at: {output_dir}\n\n"
                                f"Files created:\n" +
                                "\n".join(f"  - {f}" for f in files) +
                                f"\n\nTo run:\n  cd {output_dir}\n  npm install\n  npm run dev"
                            ),
                        }
                    ]
                }
            except Exception as e:
                return {"content": [{"type": "text", "text": f"Error: {e}"}]}

        return {"error": {"code": -32601, "message": f"Unknown tool: {tool_name}"}}

    return {"error": {"code": -32601, "message": f"Unknown method: {method}"}}


if __name__ == "__main__":
    import tempfile
    import shutil

    print("MCP React Page Generator — self-test")

    # Test tool listing
    result = handle_request({"method": "tools/list"})
    assert len(result["tools"]) == 1
    assert result["tools"][0]["name"] == "generate_react_page"
    print("[PASS] tools/list")

    # Test generation in a temp directory
    with tempfile.TemporaryDirectory() as tmpdir:
        result = handle_request({
            "method": "tools/call",
            "params": {
                "name": "generate_react_page",
                "arguments": {
                    "project_name": "test-app",
                    "title": "Test Page",
                    "description": "A test page",
                    "output_dir": tmpdir,
                },
            },
        })
        assert "content" in result
        assert "React project generated" in result["content"][0]["text"]

        # Verify files exist
        project = Path(tmpdir) / "test-app"
        assert (project / "package.json").exists()
        assert (project / "src" / "App.tsx").exists()
        assert (project / "vite.config.ts").exists()

        # Verify package.json content
        pkg = json.loads((project / "package.json").read_text())
        assert pkg["name"] == "test-app"
        assert "react" in pkg["dependencies"]
        assert "vite" in pkg["devDependencies"]

        # Verify App.tsx contains the title
        app_content = (project / "src" / "App.tsx").read_text()
        assert "Test Page" in app_content

        print("[PASS] generate_react_page")

    # Test unknown tool
    result = handle_request({
        "method": "tools/call",
        "params": {"name": "nonexistent", "arguments": {}},
    })
    assert "error" in result
    print("[PASS] unknown tool error")

    print("All tests passed.")
