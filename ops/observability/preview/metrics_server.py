#!/usr/bin/env python3
"""Serve clearly synthetic, changing Prometheus series for Grafana preview."""

import math
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


START = time.time()
LABEL_ESCAPE = str.maketrans({"\\": "\\\\", "\n": "\\n", '"': '\\"'})


def format_labels(values):
    if not values:
        return ""
    body = ",".join(
        f'{key}="{str(value).translate(LABEL_ESCAPE)}"'
        for key, value in sorted(values.items())
    )
    return "{" + body + "}"


def wave(base, amplitude, period):
    phase = (time.time() - START) / period
    return base + amplitude * (0.5 + 0.5 * math.sin(phase * math.tau))


class Metrics:
    def __init__(self):
        self.lines = []
        self.types = set()
        self.elapsed = max(0.0, time.time() - START)

    def header(self, name, metric_type):
        if name not in self.types:
            self.lines.append(f"# TYPE {name} {metric_type}")
            self.types.add(name)

    def sample(self, name, value, labels=None):
        self.lines.append(f"{name}{format_labels(labels or {})} {value:.6f}")

    def gauge(self, name, value, labels=None):
        self.header(name, "gauge")
        self.sample(name, value, labels)

    def counter(self, name, rate, labels=None, initial=0.0):
        self.header(name, "counter")
        self.sample(name, initial + rate * self.elapsed, labels)

    def histogram(self, name, rate, scale, labels=None, initial=0.0):
        self.header(name, "histogram")
        labels = labels or {}
        total = initial + rate * self.elapsed
        buckets = (0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0)
        for upper in buckets:
            fraction = -math.expm1(-upper / scale)
            self.sample(name + "_bucket", total * fraction, {**labels, "le": upper})
        self.sample(name + "_bucket", total, {**labels, "le": "+Inf"})
        self.sample(name + "_sum", total * scale, labels)
        self.sample(name + "_count", total, labels)

    def render(self, port):
        if port == 9091:
            self.app_metrics()
        else:
            self.hotpath_metrics()
        return "\n".join(self.lines) + "\n"

    def app_metrics(self):
        self.gauge("cvm_preview_fixture_info", 1, {"source": "synthetic-preview", "purpose": "grafana-dashboard-preview", "fixture_version": "1"})
        self.gauge("cvm_process_rss_bytes", wave(840_000_000, 45_000_000, 180))
        self.gauge("cvm_process_rss_anon_bytes", wave(620_000_000, 35_000_000, 210))
        self.gauge("cvm_process_swap_bytes", 0)
        self.gauge("cvm_process_threads", wave(38, 4, 150))
        self.counter("cvm_process_cpu_seconds_total", 0.42, initial=1800)
        self.gauge("cvm_storage_available_bytes", wave(68_000_000_000, 2_000_000_000, 260), {"filesystem": "data"})
        self.gauge("cvm_storage_database_bytes", wave(7_400_000_000, 230_000_000, 240), {"database": "main"})
        self.gauge("cvm_sqlite_wal_bytes", wave(38_000_000, 15_000_000, 80), {"database": "main"})
        self.gauge("cvm_projection_last_good_age_seconds", wave(7, 2, 120), {"projection": "dashboard"})
        self.gauge("cvm_maintenance_backlog_age_seconds", wave(42, 18, 150), {"operation": "retention"})
        now = time.time()
        for source, age in (("cpu", 13), ("memory", 18), ("files", 24)):
            self.gauge("cvm_observability_sampler_last_success_timestamp_seconds", now - age, {"source": source})

        endpoints = ("responses", "chat_completions")
        for endpoint_index, endpoint in enumerate(endpoints):
            for outcome_index, outcome in enumerate(("success", "error", "cancelled")):
                self.counter(
                    "cvm_proxy_invocations_total",
                    (4.4 - outcome_index * 0.8) * (1.0 - endpoint_index * 0.28),
                    {"endpoint": endpoint, "outcome": outcome},
                    initial=200 + endpoint_index * 90 + outcome_index * 12,
                )
            self.counter("cvm_proxy_retries_total", 0.18 + endpoint_index * 0.06, {"endpoint": endpoint, "reason": "timeout"}, initial=4)
            self.counter("cvm_proxy_retries_total", 0.07, {"endpoint": endpoint, "reason": "rate_limit"}, initial=2)
            for outcome_index, outcome in enumerate(("success", "error", "cancelled")):
                self.counter(
                    "cvm_proxy_upstream_attempts_total",
                    (4.8 - outcome_index * 1.0) * (1.0 - endpoint_index * 0.25),
                    {"endpoint": endpoint, "outcome": outcome},
                    initial=300 + endpoint_index * 70,
                )
            self.counter("cvm_proxy_transfer_bytes_total", 15_000 + endpoint_index * 7_000, {"endpoint": endpoint, "direction": "request"}, initial=2_000_000)
            self.counter("cvm_proxy_transfer_bytes_total", 82_000 + endpoint_index * 35_000, {"endpoint": endpoint, "direction": "response"}, initial=9_000_000)
            for phase, scale, rate in (("connect", 0.012, 4.0), ("headers", 0.035, 4.0), ("body", 0.22, 3.7), ("stream", 0.65, 3.2)):
                self.histogram("cvm_proxy_phase_duration_seconds", rate, scale, {"endpoint": endpoint, "phase": phase}, initial=80)
            self.histogram("cvm_proxy_ttft_seconds", 4.3, 0.19 + endpoint_index * 0.04, {"endpoint": endpoint}, initial=120)
            self.histogram("cvm_proxy_ttfb_seconds", 4.3, 0.26 + endpoint_index * 0.05, {"endpoint": endpoint}, initial=120)
            self.histogram("cvm_proxy_stream_duration_seconds", 3.6, 0.95 + endpoint_index * 0.16, {"endpoint": endpoint}, initial=100)

        wait_scales = {"sqlite_coordinator": .015, "db_pool": .005, "account_capacity": .12,
                       "retry_backoff": .18, "downstream_channel": .006, "terminal_priority": .009, "journal_lock": .003}
        for endpoint in endpoints:
            labels = {"endpoint": endpoint}
            for name, scale in (("response_duration", .9), ("local_wait", .045), ("unattributed", .018)):
                self.histogram("cvm_request_" + name + "_seconds", 3.4, scale, labels, initial=100)
            self.histogram("cvm_request_persistence_seconds", 3.4, .075, {**labels, "outcome": "committed"}, initial=100)
            for phase, scale in (("auth_route", .01), ("request_read", .015), ("request_parse", .005), ("attempt", .7),
                                 ("connect", .035), ("upstream_head", .285), ("forward", .47), ("finalize", .08), ("journal_append", .002)):
                self.histogram("cvm_request_stage_seconds", 3.4, scale, {**labels, "phase": phase}, initial=100)
            for resource, scale in wait_scales.items():
                self.histogram("cvm_request_resource_wait_seconds", 3.4, scale, {**labels, "resource": resource}, initial=100)
                self.counter("cvm_request_wait_affected_total", 1.0, {**labels, "resource": resource}, initial=30)
                self.counter("cvm_request_wait_over_100ms_total", .12, {**labels, "resource": resource}, initial=3)
        for resource, scale in wait_scales.items():
            self.histogram("cvm_resource_wait_event_seconds", 2.4, scale, {"resource": resource, "outcome": "complete"}, initial=80)
            self.counter("cvm_resource_wait_events_total", 2.4, {"resource": resource, "outcome": "complete"}, initial=80)
        self.counter("cvm_trace_exported_spans_total", 40.0, initial=500)
        for name in ("cvm_trace_export_failed_spans_total", "cvm_trace_queue_dropped_spans_total", "cvm_diagnostic_dropped_total", "cvm_metric_series_dropped_total"):
            self.counter(name, 0)

        for route, method, status_class, rate in (
            ("/v1/responses", "POST", "2xx", 4.8),
            ("/v1/chat/completions", "POST", "2xx", 3.1),
            ("/v1/responses", "POST", "4xx", 0.22),
            ("/v1/chat/completions", "POST", "5xx", 0.08),
            ("/health", "GET", "2xx", 0.65),
        ):
            labels = {"route": route, "method": method, "status_class": status_class}
            self.counter("cvm_http_requests_total", rate, labels, initial=500)
            self.histogram("cvm_http_header_duration_seconds", rate, 0.018 if route != "/health" else 0.004, labels, initial=200)
            self.histogram("cvm_http_body_duration_seconds", rate, 0.16 if route != "/health" else 0.002, labels, initial=200)
            self.counter("cvm_http_body_ends_total", rate, {"route": route, "outcome": "success" if status_class == "2xx" else "error"}, initial=200)

        for task_key, rate, scale in (("retention", 0.22, 7.5), ("projection", 0.45, 3.2), ("reconcile", 0.12, 11.0)):
            self.counter("cvm_task_runs_total", rate, {"task_key": task_key, "outcome": "success"}, initial=40)
            self.counter("cvm_task_runs_total", 0.015, {"task_key": task_key, "outcome": "error"}, initial=1)
            self.histogram("cvm_task_run_duration_seconds", rate, scale, {"task_key": task_key}, initial=35)

        for queue, value in (("all", 18), ("p1_terminal", 6), ("p2_derived", 12)):
            self.gauge("cvm_sqlite_pending_items", wave(value, 4, 100), {"queue": queue})
        self.gauge("cvm_sqlite_coordinator_waiters", wave(2, 1.5, 75), {"class": "write"})
        self.gauge("cvm_sqlite_coordinator_waiters", wave(1, 0.8, 90), {"class": "read"})
        self.counter("cvm_sqlite_errors_total", 0.045, {"kind": "busy"}, initial=3)
        self.counter("cvm_sqlite_errors_total", 0.012, {"kind": "locked"}, initial=1)
        self.counter("cvm_sqlite_pool_timeouts_total", 0.008, initial=1)
        self.counter("cvm_sqlite_retries_total", 0.13, {"class": "write", "reason": "lock"}, initial=5)
        self.counter("cvm_sqlite_defers_total", 0.06, {"class": "read", "reason": "pressure"}, initial=2)
        for metric, scale in (
            ("cvm_sqlite_coordinator_wait_duration_seconds", 0.032),
            ("cvm_sqlite_coordinator_hold_duration_seconds", 0.021),
            ("cvm_sqlite_pool_acquire_duration_seconds", 0.008),
            ("cvm_sqlite_queue_wait_duration_seconds", 0.045),
            ("cvm_sqlite_batch_execute_duration_seconds", 0.11),
            ("cvm_sqlite_batch_ack_duration_seconds", 0.018),
            ("cvm_terminal_enqueue_to_commit_duration_seconds", 0.075),
        ):
            self.histogram(metric, 2.3, scale, {"class": "write"}, initial=75)

        for page_index, page in enumerate(("records", "settings", "overview")):
            for device_index, device in enumerate(("desktop", "mobile")):
                labels = {"page": page, "device": device}
                self.histogram("cvm_browser_data_ready_seconds", 0.55, 0.42 + page_index * 0.06 + device_index * 0.08, labels, initial=30)
                self.histogram("cvm_browser_update_to_paint_seconds", 0.55, 0.09 + device_index * 0.03, labels, initial=30)
                self.histogram("cvm_browser_api_duration_seconds", 0.48, 0.18 + page_index * 0.02, labels, initial=25)
                self.counter("cvm_browser_events_total", 0.12, {**labels, "event": "long_task"}, initial=3)
                self.counter("cvm_browser_sse_ends_total", 0.16, {**labels, "outcome": "normal"}, initial=15)
                self.counter("cvm_browser_sse_ends_total", 0.015, {**labels, "outcome": "error"}, initial=1)
                self.counter("cvm_browser_sse_ends_total", 0.004, {**labels, "outcome": "unknown"}, initial=1)
                self.counter("cvm_browser_ingest_total", 0.22, {**labels, "outcome": "accepted", "reason": "valid"}, initial=20)
                self.counter("cvm_browser_ingest_total", 0.018, {**labels, "outcome": "rejected", "reason": "stale"}, initial=2)
                self.counter("cvm_browser_dropped_total", 0.01, labels, initial=1)
                self.counter("cvm_browser_unsupported_total", 0.004, labels, initial=1)
                self.counter("cvm_browser_visibility_hidden_total", 0.03, labels, initial=2)

    def hotpath_metrics(self):
        for function, rate, scale in (
            ("proxy::route", 6.1, 0.004),
            ("proxy::stream", 4.6, 0.018),
            ("sqlite::commit", 3.8, 0.006),
            ("records::snapshot", 1.6, 0.021),
        ):
            self.counter("hotpath_function_calls_total", rate, {"function": function}, initial=300)
            self.histogram("hotpath_function_duration_seconds", rate, scale, {"function": function}, initial=300)
        for query, rate, scale in (
            ("select_invocations", 4.4, 0.012),
            ("insert_terminal", 2.8, 0.019),
            ("refresh_projection", 1.2, 0.036),
        ):
            self.counter("hotpath_sql_queries_total", rate, {"query": query}, initial=200)
            self.histogram("hotpath_sql_duration_seconds", rate, scale, {"query": query}, initial=200)
        self.histogram("hotpath_mutex_wait_seconds", 3.0, 0.003, {"label": "sqlite-writer"}, initial=25)


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802
        if self.path == "/metrics":
            payload = Metrics().render(self.server.server_port).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/plain; version=0.0.4")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            return
        self.send_response(404)
        self.end_headers()

    def log_message(self, *_args):
        return


def serve(port):
    server = ThreadingHTTPServer(("0.0.0.0", port), Handler)
    server.serve_forever()


if __name__ == "__main__":
    import threading

    import traces
    threading.Thread(target=traces.seed, daemon=True).start()
    threading.Thread(target=traces.serve, daemon=True).start()
    threading.Thread(target=serve, args=(6772,), daemon=True).start()
    serve(9091)
