#!/usr/bin/env python3
"""
Lightweight model routing proxy for Codex CLI.

Routes Chat API requests to different backends based on the model name.
Use this when you have multiple third-party providers but Codex can only
use one `model_provider` at a time.

Usage:
    export DEEPSEEK_API_KEY="sk-xxx"
    export MIMO_API_KEY="xxx"
    python3 router.py [port]

    # In config.toml:
    # [model_providers.myrouter]
    # name = "MyRouter"
    # base_url = "http://localhost:9090/v1"
    # wire_api = "chat"
    # model_provider = "myrouter"
"""
import json
import os
import urllib.request
import urllib.error
import http.server
import sys
import threading

# ---------------------------------------------------------------------------
# Provider routing table
# ---------------------------------------------------------------------------
# Add or remove entries. The key is the model name prefix used for matching.
# The first matching prefix wins.

PROVIDERS = {
    "deepseek-": {
        "base_url": "https://api.deepseek.com/v1",
        "api_key_env": "DEEPSEEK_API_KEY",
    },
    "mimo-": {
        "base_url": "https://api.xiaomimimo.com/v1",
        "api_key_env": "MIMO_API_KEY",
    },
}

# ---------------------------------------------------------------------------
# CORS headers (needed if a browser-based tool connects)
# ---------------------------------------------------------------------------
CORS_HEADERS = {
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type, Authorization",
}


def resolve_provider(model: str):
    """Return the provider config for a given model name, or None."""
    for prefix, config in PROVIDERS.items():
        if model.startswith(prefix):
            return config
    return None


def merge_headers(base, additional):
    result = dict(base)
    result.update(additional)
    return result


class ProxyHandler(http.server.BaseHTTPRequestHandler):
    """Handles proxied Chat API requests."""

    # Silence per-request logs; we print our own.
    def log_message(self, format, *args):
        print(f"[Router] {args[0]} {args[1]} {args[2]}", file=sys.stderr)

    def _send_cors(self):
        for k, v in CORS_HEADERS.items():
            self.send_header(k, v)

    def do_OPTIONS(self):
        """Handle CORS preflight."""
        self.send_response(204)
        self._send_cors()
        self.end_headers()

    def do_GET(self):
        if self.path in ("/v1/models", "/models"):
            self._handle_models()
        else:
            self.send_error(404)

    def _handle_models(self):
        """Return the list of models this router can handle."""
        models = []
        for prefix in PROVIDERS:
            # Generate plausible model names from the prefix
            models.append({"id": prefix.rstrip("-"), "object": "model"})
        body = json.dumps({
            "object": "list",
            "data": models,
        }).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self._send_cors()
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        # Only proxy /v1/chat/completions
        if self.path not in ("/v1/chat/completions", "/chat/completions"):
            self.send_error(404)
            return

        # Read the request body
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        try:
            data = json.loads(body)
        except json.JSONDecodeError:
            self.send_error(400, "Invalid JSON")
            return

        model = data.get("model", "")
        provider = resolve_provider(model)

        if not provider:
            self.send_error(
                400,
                f"Cannot determine provider for model '{model}'. "
                f"Known prefixes: {', '.join(PROVIDERS.keys())}",
            )
            return

        api_key = os.environ.get(provider["api_key_env"])
        if not api_key:
            self.send_error(
                401,
                f"Missing {provider['api_key_env']} environment variable "
                f"for model '{model}'",
            )
            return

        target_url = provider["base_url"] + "/chat/completions"

        # Build the upstream request
        req = urllib.request.Request(
            target_url,
            data=json.dumps(data).encode(),
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {api_key}",
            },
            method="POST",
        )

        try:
            with urllib.request.urlopen(req) as resp:
                response_data = resp.read()
                self.send_response(resp.status)
                # Pass through relevant response headers
                for key in ("Content-Type", "Content-Encoding", "Transfer-Encoding"):
                    value = resp.headers.get(key)
                    if value:
                        self.send_header(key, value)
                self._send_cors()
                self.end_headers()
                self.wfile.write(response_data)
        except urllib.error.HTTPError as e:
            self.send_response(e.code)
            self.send_header("Content-Type", "application/json")
            self._send_cors()
            self.end_headers()
            self.wfile.write(e.read())
        except urllib.error.URLError as e:
            self.send_error(502, f"Upstream connection error: {e.reason}")
        except OSError as e:
            self.send_error(502, f"Upstream error: {e}")


def main():
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 9090
    server = http.server.HTTPServer(("127.0.0.1", port), ProxyHandler)

    print(f"🚀  Model router proxy listening on http://localhost:{port}/v1", file=sys.stderr)
    for prefix, config in PROVIDERS.items():
        key_status = "✅" if os.environ.get(config["api_key_env"]) else "❌"
        print(f"   {prefix:<15} → {config['base_url']:<45} {key_status} ${config['api_key_env']}", file=sys.stderr)
    print(file=sys.stderr)
    print(f"   Config for ~/.codex/config.toml:", file=sys.stderr)
    print(f"     [model_providers.myrouter]", file=sys.stderr)
    print(f'     name = "MyRouter"', file=sys.stderr)
    print(f'     base_url = "http://localhost:{port}/v1"', file=sys.stderr)
    print(f'     wire_api = "chat"', file=sys.stderr)
    print(f"     model_provider = \"myrouter\"", file=sys.stderr)
    print(file=sys.stderr)

    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nShutting down.", file=sys.stderr)
        server.shutdown()


if __name__ == "__main__":
    main()
