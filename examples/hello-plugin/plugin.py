#!/usr/bin/env python3
"""hello-plugin: the smallest plugin of Vumlo (ADR-0066). Python standard library only.

It subscribes to `submission.submitted`. When one arrives it fetches the submission through the
plugin API, counts the words, and posts a report back. It shows the whole protocol:

  GET  /manifest   what the plugin is and asks for (read once, when an administrator installs it)
  POST /events     a signed event from the core: check the signature and the time, answer 2xx at
                   once, work afterwards
  ...and, towards the core, `GET /plugin-api/submissions/{id}` and `PUT .../report` with the token.

Environment:
  PORT                  where to listen on 127.0.0.1 (0: pick a free port and print it)
  VUMLO_API             the API of the core, e.g. http://127.0.0.1:8080/api/v1
  VUMLO_PLUGIN_TOKEN    printed by `vumlo-cli plugin install`
  VUMLO_WEBHOOK_SECRET  printed by `vumlo-cli plugin install`
The last three are not known until the plugin is installed, so start it without them first, install
it, then start it again with them. Without them it serves the manifest and refuses events.
"""

import hashlib
import hmac
import json
import os
import sys
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MANIFEST = {
    "id": "hello",
    "version": "1.0.0",
    "api": "1",
    "subscribes": ["submission.submitted"],
    "scopes": ["submissions:read", "reports:write"],
    "ui": [],
}
# An event older (or newer) than this is refused: a captured request cannot be replayed later.
MAX_AGE_SECONDS = 300

API = os.environ.get("VUMLO_API", "").rstrip("/")
TOKEN = os.environ.get("VUMLO_PLUGIN_TOKEN", "")
SECRET = os.environ.get("VUMLO_WEBHOOK_SECRET", "")

seen = set()  # event ids already taken; the core delivers at least once, so repeats come
seen_lock = threading.Lock()


def signature(secret, timestamp, body):
    """`sha256=` and the hex HMAC-SHA256 of `timestamp + "." + body`, as the core sends it."""
    message = timestamp.encode() + b"." + body
    return "sha256=" + hmac.new(secret.encode(), message, hashlib.sha256).hexdigest()


def check(secret, headers, body, now=None):
    """None when the event is genuine and fresh, else (status, why)."""
    if not secret:
        return 503, "no webhook secret configured"
    timestamp = headers.get("X-Vumlo-Timestamp", "")
    try:
        sent_at = int(timestamp)
    except ValueError:
        return 401, "bad timestamp"
    if abs((now if now is not None else time.time()) - sent_at) > MAX_AGE_SECONDS:
        return 401, "stale timestamp"
    # compare_digest: the comparison takes the same time whatever the first difference is.
    if not hmac.compare_digest(signature(secret, timestamp, body), headers.get("X-Vumlo-Signature", "")):
        return 401, "bad signature"
    return None


def count_words(node):
    """The words in the `text` leaves of a block document."""
    if isinstance(node, dict):
        own = len(node["text"].split()) if node.get("type") == "text" and isinstance(node.get("text"), str) else 0
        return own + sum(count_words(value) for value in node.values())
    if isinstance(node, list):
        return sum(count_words(item) for item in node)
    return 0


def call(method, path, payload=None):
    request = urllib.request.Request(
        API + path,
        method=method,
        data=None if payload is None else json.dumps(payload).encode(),
        headers={"Authorization": "Bearer " + TOKEN, "Content-Type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=10) as response:
        body = response.read()
        return json.loads(body) if body else None


def work(event):
    """Fetch the submission, count its words, report. Runs after the core was answered."""
    try:
        submission_id = event["data"]["submission_id"]
        submission = call("GET", "/plugin-api/submissions/" + submission_id)
        words = count_words(submission["content"])
        call("PUT", "/plugin-api/submissions/%s/report" % submission_id,
             {"summary": "%d words" % words, "data": {"words": words}})
        print("reported %d words for %s" % (words, submission_id), flush=True)
    except (urllib.error.URLError, KeyError, ValueError) as error:
        # An HTTP error carries its status (403: the scope was not granted).
        print("could not report: %s" % error, file=sys.stderr, flush=True)


class Handler(BaseHTTPRequestHandler):
    def reply(self, status, payload=None):
        body = b"" if payload is None else json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/manifest":
            self.reply(200, MANIFEST)
        else:
            self.reply(404, {"error": "not found"})

    def do_POST(self):
        if self.path != "/events":
            return self.reply(404, {"error": "not found"})
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        refused = check(SECRET, self.headers, body)
        if refused:
            return self.reply(refused[0], {"error": refused[1]})
        try:
            event = json.loads(body)
            event_id, kind = event["id"], event["type"]
        except (ValueError, KeyError, TypeError):
            return self.reply(400, {"error": "not an event"})
        with seen_lock:
            first = event_id not in seen
            seen.add(event_id)
        if first and kind == "submission.submitted":
            threading.Thread(target=work, args=(event,), daemon=True).start()
        self.reply(202)  # taken; the work goes on without the core waiting

    def log_message(self, format, *args):  # keep the request line out of the log
        pass


def check_vector():
    """The vector the core's tests use for its signature (backend/crates/plugins)."""
    body = b'{"id":"e1","type":"submission.submitted","data":{"submission_id":"s1"}}'
    expected = "sha256=93d5d9c8289fe4a0066a2634f57443ea75081452aec5ad86e7310daeaf07e941"
    assert signature("test-secret", "1700000000", body) == expected, "the signature differs from the core's"
    headers = {"X-Vumlo-Timestamp": "1700000000", "X-Vumlo-Signature": expected}
    assert check("test-secret", headers, body, now=1700000000) is None
    assert check("test-secret", headers, body, now=1700000000 + MAX_AGE_SECONDS + 1) == (401, "stale timestamp")
    headers["X-Vumlo-Signature"] = expected[:-1] + "0"
    assert check("test-secret", headers, body, now=1700000000) == (401, "bad signature")
    print("ok")


if __name__ == "__main__":
    if "--check-vector" in sys.argv:
        check_vector()
        sys.exit(0)
    server = ThreadingHTTPServer(("127.0.0.1", int(os.environ.get("PORT", "0"))), Handler)
    print("listening on http://127.0.0.1:%d" % server.server_address[1], flush=True)
    server.serve_forever()
