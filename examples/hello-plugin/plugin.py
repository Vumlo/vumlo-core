#!/usr/bin/env python3
"""hello-plugin: the smallest plugin of Vumlo (ADR-0066). Python standard library only.

It subscribes to `submission.submitted`. When one arrives it fetches the submission through the
plugin API, counts the words, and posts a report back. It shows the whole protocol:

  GET  /manifest   what the plugin is and asks for (read once, when an administrator installs it)
  POST /events     a signed event from the core: check the signature and the time, answer 2xx at
                   once, work afterwards
  ...and, towards the core, `GET /plugin-api/submissions/{id}` and `PUT .../report` with the token.

And a page for the tab "Words" on an assignment:

  GET  /ui/words   the page the web app shows in a sandboxed frame (an opaque origin, no cookie).
                   It sends its own strict CSP. The page asks the web app who is looking
                   (`vumlo.ready`), gets a launch token (`vumlo.context`), and sends it here:
  GET  /ui/data    checks the token with the webhook secret, then answers by who looks: the totals for
                   the staff of the course, nothing for a student. CORS, because the frame's origin
                   is `null`.

Environment:
  PORT                  where to listen on 127.0.0.1 (0: pick a free port and print it)
  PUBLIC_URL            where the core reaches this plugin; the page may call only this (default:
                        http://127.0.0.1:PORT)
  VUMLO_ORIGIN          the origin of the web app; when set, only it may frame the page
  VUMLO_API             the API of the core, e.g. http://127.0.0.1:8080/api/v1
  VUMLO_PLUGIN_TOKEN    printed by `vumlo-cli plugin install`
  VUMLO_WEBHOOK_SECRET  printed by `vumlo-cli plugin install`
The last three are not known until the plugin is installed, so start it without them first, install
it, then start it again with them. Without them it serves the manifest and refuses events.
"""

import base64
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
    "ui": [{"slot": "assignment.tab", "title": "Words", "path": "/ui/words"}],
}
# An event older (or newer) than this is refused: a captured request cannot be replayed later.
MAX_AGE_SECONDS = 300

API = os.environ.get("VUMLO_API", "").rstrip("/")
TOKEN = os.environ.get("VUMLO_PLUGIN_TOKEN", "")
SECRET = os.environ.get("VUMLO_WEBHOOK_SECRET", "")

seen = set()  # event ids already taken; the core delivers at least once, so repeats come
# What the page shows to staff: per assignment, how many works and how many words in all.
stats = {}
seen_lock = threading.Lock()
PUBLIC_URL = os.environ.get("PUBLIC_URL", "").rstrip("/")  # filled in from the port when unset
VUMLO_ORIGIN = os.environ.get("VUMLO_ORIGIN", "").rstrip("/")


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


def launch_claims(secret, token, now=None):
    """The claims of a launch token the core signed for this plugin, or None.

    The token is `base64url(payload) + "." + hex(HMAC-SHA256(secret, "vumlo-launch." + base64url(payload)))`.
    The prefix keeps it apart from the signature of an event, so one cannot pass for the other.
    """
    if not secret or not isinstance(token, str) or token.count(".") != 1:
        return None
    payload, signature = token.split(".")
    expected = hmac.new(secret.encode(), b"vumlo-launch." + payload.encode(), hashlib.sha256).hexdigest()
    if not hmac.compare_digest(expected, signature):
        return None
    try:
        claims = json.loads(base64.urlsafe_b64decode(payload + "=" * (-len(payload) % 4)))
        if claims["v"] != 1 or claims["plugin"] != MANIFEST["id"]:
            return None
        if (now if now is not None else time.time()) >= claims["exp"]:
            return None
        return claims
    except (ValueError, KeyError, TypeError):
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
        with seen_lock:
            total = stats.setdefault(submission["assignment_id"], {"submissions": 0, "words": 0})
            total["submissions"] += 1
            total["words"] += words
        call("PUT", "/plugin-api/submissions/%s/report" % submission_id,
             {"summary": "%d words" % words, "data": {"words": words}})
        print("reported %d words for %s" % (words, submission_id), flush=True)
    except (urllib.error.URLError, KeyError, ValueError) as error:
        # An HTTP error carries its status (403: the scope was not granted).
        print("could not report: %s" % error, file=sys.stderr, flush=True)


# The page of the tab. It runs in a frame with an opaque origin: no cookie, no storage, nothing of the
# web app. It draws with the colours the web app sends (`tokens`), so it looks like the rest.
PAGE_STYLE = """
html { background: var(--surface, #fff); color: var(--text, #111); }
body { margin: 0; padding: 16px; font: 14px/1.5 system-ui, sans-serif; }
.card { border: 1px solid var(--border, #ccc); border-radius: 10px; background: var(--sunken, #f6f6f6); padding: 12px 16px; }
.big { font-size: 28px; font-weight: 600; color: var(--accent, #36c); }
.muted { color: var(--muted, #555); }
"""

PAGE_SCRIPT = """
const out = document.getElementById("out");
function show(parts) {
  out.replaceChildren(...parts.map(([cls, text]) => {
    const p = document.createElement("p");
    p.className = cls;
    p.textContent = text;
    return p;
  }));
  // Tell the web app how tall the page is, so the frame has no scrollbar of its own.
  parent.postMessage({ type: "vumlo.resize", v: 1, height: document.documentElement.scrollHeight }, "*");
}
addEventListener("message", async (event) => {
  // Only the web app that framed this page; anything else is somebody else's message.
  if (event.source !== parent) return;
  const m = event.data;
  if (!m || m.v !== 1 || m.type !== "vumlo.context") return;
  const root = document.documentElement;
  root.lang = m.language;
  root.dataset.theme = m.theme;
  for (const [name, value] of Object.entries(m.tokens)) root.style.setProperty(name, value);
  try {
    const res = await fetch("/ui/data", { headers: { Authorization: "Launch " + m.launch.token } });
    const data = await res.json();
    if (!res.ok) return show([["muted", "Not available."]]);
    if (data.role !== "staff") return show([["muted", "Word counts are shown to the teachers of the course."]]);
    show([["big", String(data.words)], ["muted", "words in " + data.submissions + " submitted work(s)"]]);
  } catch {
    show([["muted", "The plugin did not answer."]]);
  }
});
parent.postMessage({ type: "vumlo.ready", v: 1 }, "*");
"""


def sha256_source(text):
    """A CSP source for exactly this inline text."""
    return "'sha256-" + base64.b64encode(hashlib.sha256(text.encode()).digest()).decode() + "'"


def page():
    return (
        '<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Words</title>'
        "<style>%s</style></head><body><main id=\"out\"><p class=\"muted\">…</p></main>"
        "<script>%s</script></body></html>" % (PAGE_STYLE, PAGE_SCRIPT)
    )


def page_csp():
    """Strict: nothing loads, and only this exact script and style run. The page may call only this
    plugin. `frame-ancestors` names the web app when it is known: nobody else may frame the page."""
    rules = [
        "default-src 'none'",
        "script-src " + sha256_source(PAGE_SCRIPT),
        "style-src " + sha256_source(PAGE_STYLE),
        "connect-src " + (PUBLIC_URL or "'self'"),
        "base-uri 'none'",
        "form-action 'none'",
    ]
    if VUMLO_ORIGIN:
        rules.append("frame-ancestors " + VUMLO_ORIGIN)
    return "; ".join(rules)


class Handler(BaseHTTPRequestHandler):
    def reply(self, status, payload=None, headers=()):
        body = b"" if payload is None else json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        for name, value in headers:
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(body)

    def reply_page(self):
        body = page().encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Content-Security-Policy", page_csp())
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "no-referrer")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/manifest":
            self.reply(200, MANIFEST)
        elif self.path == "/ui/words":
            self.reply_page()
        elif self.path == "/ui/data":
            self.reply_data()
        else:
            self.reply(404, {"error": "not found"})

    def reply_data(self):
        """Who looks, from the launch token the core signed: totals for staff, nothing for a student."""
        cors = [("Access-Control-Allow-Origin", "*")]  # the frame's origin is `null`
        scheme, _, token = self.headers.get("Authorization", "").partition(" ")
        claims = launch_claims(SECRET, token) if scheme == "Launch" else None
        if claims is None:
            return self.reply(401, {"error": "bad or expired token"}, cors)
        if claims["role"] != "staff":
            return self.reply(200, {"role": "student"}, cors)
        with seen_lock:
            total = dict(stats.get(claims["assignment"], {"submissions": 0, "words": 0}))
        self.reply(200, {"role": "staff", **total}, cors)

    def do_OPTIONS(self):
        # The preflight of the call above: it carries an Authorization header.
        self.reply(204, None, [
            ("Access-Control-Allow-Origin", "*"),
            ("Access-Control-Allow-Headers", "authorization"),
            ("Access-Control-Allow-Methods", "GET"),
        ])

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
    # A launch token, signed by the core with the same secret (backend/crates/plugins, secret tests).
    token = (
        "eyJhc3NpZ25tZW50IjoiMDE4ZjAwMDAtMDAwMC03MDAwLTgwMDAtMDAwMDAwMDAwMDAyIiwiZXhwIjoxNzAwMDAwMDYwLCJp"
        "YXQiOjE3MDAwMDAwMDAsInBsdWdpbiI6ImhlbGxvIiwicm9sZSI6InN0YWZmIiwidXNlciI6IjAxOGYwMDAwLTAwMDAtNzAw"
        "MC04MDAwLTAwMDAwMDAwMDAwMyIsInYiOjF9."
        "9dfee57a6b11ea16056625ebbc8e28cb9a32e7fada4f7a630218fff1996bc9c0"
    )
    claims = launch_claims("test-secret", token, now=1700000010)
    assert claims and claims["role"] == "staff" and claims["plugin"] == "hello", "the token differs from the core's"
    assert launch_claims("test-secret", token, now=1700000060) is None, "an expired token was accepted"
    assert launch_claims("other-secret", token, now=1700000010) is None, "another secret was accepted"
    payload, token_signature = token.split(".")
    assert launch_claims("test-secret", payload[:-1] + "A." + token_signature, now=1700000010) is None, "a changed payload"
    # The signature of an event over the same bytes is not a token.
    as_event = hmac.new(b"test-secret", b"1700000000." + payload.encode(), hashlib.sha256).hexdigest()
    assert launch_claims("test-secret", payload + "." + as_event, now=1700000010) is None, "an event passed as a token"
    print("ok")


if __name__ == "__main__":
    if "--check-vector" in sys.argv:
        check_vector()
        sys.exit(0)
    server = ThreadingHTTPServer(("127.0.0.1", int(os.environ.get("PORT", "0"))), Handler)
    if not PUBLIC_URL:
        PUBLIC_URL = "http://127.0.0.1:%d" % server.server_address[1]
    print("listening on http://127.0.0.1:%d" % server.server_address[1], flush=True)
    server.serve_forever()
