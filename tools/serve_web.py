#!/usr/bin/env python3
"""Serve the web export locally.

The threaded Godot web build needs SharedArrayBuffer, which browsers only
enable when the page is cross-origin isolated (COOP + COEP headers).

Usage: python3 tools/serve_web.py [port] [directory]
"""
import sys
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

DEFAULT_PORT = 8060
DEFAULT_DIRECTORY = Path(__file__).resolve().parent.parent / "site"


class CrossOriginIsolatedHandler(SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


def main():
    port = int(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_PORT
    directory = sys.argv[2] if len(sys.argv) > 2 else str(DEFAULT_DIRECTORY)
    handler = partial(CrossOriginIsolatedHandler, directory=directory)
    print(f"Serving {directory} at http://localhost:{port}")
    ThreadingHTTPServer(("localhost", port), handler).serve_forever()


if __name__ == "__main__":
    main()
