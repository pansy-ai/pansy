#!/usr/bin/env python3
"""A vendor that answers nothing: it writes down who asked for which model
and refuses, so an agent started against it ends at once."""

import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = sys.argv[2]


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def note(self, body):
        model = ""
        try:
            model = json.loads(body or b"{}").get("model", "")
        except ValueError:
            pass
        key = (
            self.headers.get("authorization")
            or self.headers.get("x-api-key")
            or self.headers.get("x-goog-api-key")
            or ""
        )
        with open(LOG, "a") as log:
            log.write(f"{self.server.server_port} {self.command} {self.path} model={model} key={key}\n")

    def refuse(self):
        body = json.dumps({"error": {"message": "the spike's vendor answers nothing", "type": "invalid_request_error"}}).encode()
        self.send_response(400)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        self.note(self.rfile.read(int(self.headers.get("content-length") or 0)))
        self.refuse()

    def do_GET(self):
        self.note(b"")
        self.refuse()


ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
