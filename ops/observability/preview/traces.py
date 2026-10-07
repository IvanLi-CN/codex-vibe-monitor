"""Private Tempo query bridge and explicitly synthetic waterfall fixtures."""
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import sys
import time
import urllib.error
import urllib.request

sys.path.insert(0, "/observability")
from tempo_access import query_route

SELECTED_TRACE = "11111111111111111111111111111111"

def fixtures(now=None):
    now = now or time.time_ns()
    def attr(key, value):
        kind = "boolValue" if isinstance(value, bool) else "doubleValue" if isinstance(value, float) else "stringValue"
        return {"key": key, "value": {kind: value}}
    def span(trace_id, span_id, name, start, end, attributes, parent=None):
        item = {"traceId": trace_id, "spanId": span_id, "name": name, "kind": 2,
                "startTimeUnixNano": str(start), "endTimeUnixNano": str(end),
                "attributes": [attr(k, v) for k, v in attributes.items()]}
        if parent: item["parentSpanId"] = parent
        return item
    spans = []
    for index, category in enumerate(("normal", "slow", "wait", "retry", "error"), 1):
        trace_id = str(index) * 32
        root_id = str(index) * 16
        duration = 31_000_000_000 if category == "slow" else 900_000_000
        start = now - duration
        attributes = {"cvm.record": "response", "cvm.endpoint": "responses", "cvm.fixture": True,
                      "cvm.outcome": "error" if category == "error" else "complete",
                      "cvm.status_class": "5xx" if category == "error" else "2xx",
                      "cvm.normal_candidate": category == "normal", "cvm.slow": category == "slow",
                      "cvm.high_wait": category == "wait", "cvm.retry": category == "retry",
                      "cvm.response_ms": duration / 1e6, "cvm.local_wait_ms": 300.0 if category == "wait" else 45.0,
                      "cvm.detail_truncated": False, "cvm.export_completeness": "unknown", "cvm.persistence": "pending"}
        spans.append(span(trace_id, root_id, "cvm.proxy.response", start, now, attributes))
        intervals = [("auth_route", 0, 10), ("request_read", 10, 25), ("request_parse", 25, 30),
                     ("attempt", 30, 700), ("connect", 30, 65), ("upstream_head", 65, 350),
                     ("forward", 350, 820), ("finalize", 820, 900)]
        for child_index, (name, begin, end) in enumerate(intervals, 20):
            spans.append(span(trace_id, f"{child_index:016x}", name, start + begin * 1_000_000,
                              start + end * 1_000_000, {"cvm.phase": name}, root_id))
        spans.append(span(trace_id, "0000000000000030", "cvm.resource.wait", start + 30_000_000,
                          start + 75_000_000, {"cvm.resource": "sqlite_coordinator", "cvm.selected_interval": True}, root_id))
        spans.append(span(trace_id, "0000000000000040", "cvm.terminal.enqueue_to_commit", now - 50_000_000,
                          now + 100_000_000, {"cvm.record": "persistence", "cvm.outcome": "committed",
                                              "cvm.shared_batch": True, "cvm.exclusive_cost": False}, root_id))
    return {"resourceSpans": [{"resource": {"attributes": [attr("service.name", "codex-vibe-monitor"),
            attr("deployment.environment.name", "synthetic-preview"), attr("service.instance.id", "fixture")]},
            "scopeSpans": [{"scope": {"name": "cvm.synthetic.preview"}, "spans": spans}]}]}

def seed():
    # Container restarts reuse the same Tempo. Do not append new timestamps to
    # deterministic IDs, which would merge unrelated fixture runs into one trace.
    existing = urllib.request.Request("http://tempo:3200/api/v2/traces/" + SELECTED_TRACE,
                                     headers={"X-Scope-OrgID": "cvm"})
    try:
        with urllib.request.urlopen(existing, timeout=2) as response:
            if response.status == 200 and json.load(response).get("trace"): return
    except (OSError, urllib.error.URLError): pass
    payload = json.dumps(fixtures()).encode()
    # Only startup admission is retried; the application exporter has no retry.
    for _ in range(60):
        try:
            request = urllib.request.Request("http://tempo:4318/v1/traces", data=payload,
                      headers={"Content-Type": "application/json", "X-Scope-OrgID": "cvm"})
            with urllib.request.urlopen(request, timeout=2) as response:
                if response.status == 200: return
        except (OSError, urllib.error.URLError): pass
        time.sleep(1)
    raise RuntimeError("synthetic trace fixture admission unavailable")

class QueryHandler(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802
        if self.path != "/api/status/buildinfo":
            try: query_route(self.path, fixed_cases=False)
            except ValueError: self.send_error(400); return
        headers = {"X-Scope-OrgID": "cvm"}
        if self.headers.get("Accept") in ("application/protobuf", "application/json"):
            headers["Accept"] = self.headers["Accept"]
        request = urllib.request.Request("http://tempo:3200" + self.path, headers=headers)
        try:
            with urllib.request.urlopen(request, timeout=5) as response:
                body = response.read(1024 * 1024 + 1)
                if len(body) > 1024 * 1024: self.send_error(502); return
                self.send_response(response.status)
                self.send_header("Content-Type", response.headers.get("Content-Type", "application/json"))
                self.send_header("Content-Length", str(len(body)))
                self.end_headers(); self.wfile.write(body)
        except urllib.error.HTTPError as error:
            self.send_error(error.code)
        except OSError: self.send_error(502)
    def log_message(self, *_args): pass

def serve():
    ThreadingHTTPServer(("0.0.0.0", 8040), QueryHandler).serve_forever()
