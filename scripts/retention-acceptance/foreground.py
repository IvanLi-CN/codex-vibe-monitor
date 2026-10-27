"""Drive measured work through the production HTTP handlers in the app's network namespace."""

from __future__ import annotations

from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import math
import re
import threading
import time
from typing import Any
import urllib.error
import urllib.parse
import urllib.request


POOL_TOKEN = "retention-acceptance-pool-token"
ACCOUNT_KEYS = ("retention-acceptance-upstream-1", "retention-acceptance-upstream-2")
BASE_URL = "http://127.0.0.1:8080"
MOCK_URL = "http://127.0.0.1:18080/"
INVOCATION_RATE = 0.673
READ_PATH = "/api/pool/upstream-accounts?includeAll=1&pageSize=50"
WRITE_PATH = "/v1/responses"
SEQUENCE_PATTERN = re.compile(r"retention-acceptance-sequence-(\d{5})")


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def request_json(path: str, method: str = "GET", payload: Any = None, token: str | None = None) -> tuple[int, bytes, Any]:
    headers = {"Accept": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    data = None
    if payload is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(payload, separators=(",", ":")).encode()
    request = urllib.request.Request(BASE_URL + path, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=15) as response:
            body = response.read()
            return response.status, body, response.headers
    except urllib.error.HTTPError as error:
        return error.code, error.read(), error.headers


class MockUpstream(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self) -> None:
        super().__init__(("127.0.0.1", 18080), MockUpstreamHandler)
        self.retry_sequences: set[int] = set()
        self.failed_sequences: set[int] = set()
        self.request_count = 0
        self.retryable_failures = 0
        self.successful_responses = 0
        self.stats_lock = threading.Lock()


class MockUpstreamHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_: Any) -> None:
        pass

    def _respond(self, status: int, body: bytes, content_type: str, headers: dict[str, str] | None = None) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self) -> None:
        try:
            length = int(self.headers.get("Content-Length", "0"))
            payload = json.loads(self.rfile.read(length))
            serialized = json.dumps(payload, ensure_ascii=True)
            match = SEQUENCE_PATTERN.search(serialized)
            if match is None:
                self._respond(400, b'{"error":"missing acceptance sequence"}', "application/json")
                return
            sequence = int(match.group(1))
        except (TypeError, ValueError, json.JSONDecodeError):
            self._respond(400, b'{"error":"invalid acceptance request"}', "application/json")
            return

        server = self.server
        assert isinstance(server, MockUpstream)
        with server.stats_lock:
            should_retry = sequence in server.retry_sequences and sequence not in server.failed_sequences
            server.request_count += 1
            if should_retry:
                server.failed_sequences.add(sequence)
                server.retryable_failures += 1
            else:
                server.successful_responses += 1
        if should_retry:
            self._respond(
                503,
                b'{"error":{"type":"server_error","message":"retryable acceptance fixture"}}',
                "application/json",
                {"Retry-After": "0"},
            )
            return

        result = {
            "id": f"resp_retention_{sequence:05d}",
            "object": "response",
            "model": "gpt-5-codex",
            "status": "completed",
            "output": [],
            "usage": {"input_tokens": 12, "output_tokens": 4, "total_tokens": 16},
        }
        if payload.get("stream"):
            event = {"type": "response.completed", "response": result}
            body = ("event: response.completed\ndata: " + json.dumps(event, separators=(",", ":")) + "\n\n").encode()
            self._respond(200, body, "text/event-stream")
        else:
            self._respond(200, json.dumps(result, separators=(",", ":")).encode(), "application/json")


def require_ok(path: str, method: str, payload: Any) -> None:
    status, body, _ = request_json(path, method, payload, POOL_TOKEN)
    if status not in (200, 201):
        raise RuntimeError(f"fixture setup failed for {path}: status={status} body={body[:300]!r}")


def seed_pool() -> None:
    require_ok("/api/pool/routing-settings", "PUT", {"apiKey": POOL_TOKEN})
    for index, api_key in enumerate(ACCOUNT_KEYS, start=1):
        require_ok(
            "/api/pool/upstream-accounts/api-keys",
            "POST",
            {
                "displayName": f"Retention acceptance account {index}",
                "apiKey": api_key,
                "boundProxyKeys": ["__direct__"],
                "upstreamBaseUrl": MOCK_URL,
            },
        )
    status, body, _ = request_json(READ_PATH, token=POOL_TOKEN)
    if status != 200:
        raise RuntimeError(f"pool read preflight failed: status={status} body={body[:300]!r}")


def response_request(sequence: int) -> str:
    payload = {
        "model": "gpt-5-codex",
        "input": f"retention-acceptance-sequence-{sequence:05d}",
        "stream": False,
        "max_output_tokens": 16,
    }
    encoded = json.dumps(payload, separators=(",", ":")).encode()
    request = urllib.request.Request(
        BASE_URL + WRITE_PATH,
        data=encoded,
        method="POST",
        headers={
            "Authorization": f"Bearer {POOL_TOKEN}",
            "Content-Type": "application/json",
            "Accept": "application/json",
        },
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        body = response.read()
        if response.status != 200:
            raise RuntimeError(f"proxy returned status {response.status}: {body[:300]!r}")
        invoke_id = response.headers.get("x-cvm-invoke-id")
    if not invoke_id:
        raise RuntimeError("proxy response omitted x-cvm-invoke-id")
    return invoke_id


def write_sample(sequence: int, start_mono: float) -> dict[str, Any]:
    scheduled_offset_ms = sequence / INVOCATION_RATE * 1000
    dispatched_offset_ms = (time.monotonic() - start_mono) * 1000
    invoke_id = response_request(sequence)
    completed_offset_ms = (time.monotonic() - start_mono) * 1000
    return {
        "sequence": sequence,
        "invoke_id": invoke_id,
        "observed_at": utc_now(),
        "scheduled_offset_ms": scheduled_offset_ms,
        "dispatched_offset_ms": dispatched_offset_ms,
        "completed_offset_ms": completed_offset_ms,
        "dispatch_lag_ms": dispatched_offset_ms - scheduled_offset_ms,
        "request_elapsed_ms": completed_offset_ms - dispatched_offset_ms,
        # Keep the arrival plan fixed when a previous request delays this dispatch.
        "elapsed_ms": completed_offset_ms - scheduled_offset_ms,
        "dataset": "codex_invocations",
        "transport": "http",
        "path": WRITE_PATH,
    }


def run_workload(run_seconds: int, retry_count: int) -> dict[str, Any]:
    expected_invocations = math.ceil(INVOCATION_RATE * run_seconds)
    if retry_count < 0 or retry_count > expected_invocations:
        raise ValueError("retry count must fit within the invocation workload")
    retry_sequences = {
        math.floor(index * expected_invocations / retry_count)
        for index in range(retry_count)
    } if retry_count else set()
    with MOCK_SERVER.stats_lock:
        MOCK_SERVER.retry_sequences = retry_sequences
    stop = threading.Event()
    failures: list[BaseException] = []
    lock = threading.Lock()
    samples: dict[str, list[dict[str, Any]]] = {"read": [], "write": []}
    invocations: list[dict[str, Any]] = []
    start_mono = time.monotonic()
    input_start = utc_now()
    deadline = start_mono + run_seconds

    def reader() -> None:
        while not stop.is_set():
            started = time.perf_counter()
            try:
                status, body, _ = request_json(READ_PATH, token=POOL_TOKEN)
                if status != 200:
                    raise RuntimeError(f"pool read returned status {status}: {body[:300]!r}")
                sample = {
                    "observed_at": utc_now(),
                    "elapsed_ms": (time.perf_counter() - started) * 1000,
                    "dataset": "upstream_accounts",
                    "transport": "http",
                    "path": "/api/pool/upstream-accounts",
                }
                with lock:
                    samples["read"].append(sample)
            except BaseException as error:
                failures.append(error)
                stop.set()
                return
            stop.wait(0.25)

    def writer() -> None:
        for sequence in range(expected_invocations):
            scheduled = start_mono + sequence / INVOCATION_RATE
            if stop.wait(max(0.0, scheduled - time.monotonic())):
                return
            try:
                sample = write_sample(sequence, start_mono)
                with lock:
                    invocations.append({"sequence": sequence, "invoke_id": sample["invoke_id"]})
                    samples["write"].append(sample)
            except BaseException as error:
                failures.append(error)
                stop.set()
                return

    reader_thread = threading.Thread(target=reader, name="retention-api-reader", daemon=True)
    writer_thread = threading.Thread(target=writer, name="retention-api-writer", daemon=True)
    reader_thread.start()
    writer_thread.start()
    while time.monotonic() < deadline and not stop.is_set():
        time.sleep(min(0.1, max(0.0, deadline - time.monotonic())))
    if not failures:
        while time.monotonic() < deadline:
            time.sleep(min(0.1, max(0.0, deadline - time.monotonic())))
    stop.set()
    writer_thread.join(timeout=5)
    reader_thread.join(timeout=5)
    input_end = utc_now()
    if failures:
        raise RuntimeError(f"foreground HTTP workload failed: {failures[0]}") from failures[0]
    if writer_thread.is_alive() or reader_thread.is_alive():
        raise TimeoutError("foreground HTTP workload did not stop at the 120-second boundary")
    if len(invocations) != expected_invocations:
        raise RuntimeError(f"proxy invocation count was {len(invocations)}, expected {expected_invocations}")
    if len(samples["read"]) < 20:
        raise RuntimeError("foreground pool API reader produced fewer than 20 samples")

    server = MOCK_SERVER
    with server.stats_lock:
        mock_stats = {
            "request_count": server.request_count,
            "retryable_failures": server.retryable_failures,
            "successful_responses": server.successful_responses,
        }
    expected_attempts = expected_invocations + retry_count
    if mock_stats != {
        "request_count": expected_attempts,
        "retryable_failures": retry_count,
        "successful_responses": expected_invocations,
    }:
        raise RuntimeError(f"mock upstream traffic was {mock_stats}, expected {expected_attempts} attempts and {retry_count} retries")
    return {
        "input_started_at": input_start,
        "input_finished_at": input_end,
        "latency_samples_ms": samples,
        "invocations": sorted(invocations, key=lambda item: item["sequence"]),
        "foreground_workload": {
            "invocation_requests": len(invocations),
            "pool_read_requests": len(samples["read"]),
            "mock_upstream": mock_stats,
        },
    }


def main() -> int:
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--run-seconds", type=int, default=120)
    parser.add_argument("--retry-count", type=int, default=16)
    args = parser.parse_args()
    global MOCK_SERVER
    MOCK_SERVER = MockUpstream()
    server_thread = threading.Thread(target=MOCK_SERVER.serve_forever, name="retention-mock-upstream", daemon=True)
    server_thread.start()
    try:
        seed_pool()
        print(json.dumps(run_workload(args.run_seconds, args.retry_count), sort_keys=True), flush=True)
        return 0
    finally:
        MOCK_SERVER.shutdown()
        MOCK_SERVER.server_close()
        server_thread.join(timeout=2)


if __name__ == "__main__":
    raise SystemExit(main())
