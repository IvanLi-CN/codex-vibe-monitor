#!/usr/bin/env python3
"""Forward proxy algorithm backtest against a local SQLite snapshot.

The script compares baseline v1 and candidate v2 using:
1) Trace replay over recorded forward_proxy_attempts in chronological order.
2) Stochastic simulation from distributions estimated on the same database.

Security defaults:
- Database is opened with sqlite read-only URI mode.
- Database path inside repository is rejected.
- Reports are written under /tmp by default.
- Proxy identifiers are redacted by default.
"""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import hashlib
import json
import math
import random
import sqlite3
import statistics
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Tuple

DIRECT_KEY = "__direct__"


@dataclass(frozen=True)
class AlgoConfig:
    name: str
    success_bonus: float
    latency_divisor: float
    latency_cap: float
    success_min_gain: Optional[float]
    failure_base: float
    failure_step: float
    failure_uses_offset: bool
    failure_cap: Optional[float]
    weight_min: float
    weight_max: float
    success_floor: float
    probe_every_requests: int
    probe_interval_secs: int
    probe_recovery_weight: float
    min_positive_candidates: int
    direct_initial_weight: float


ALGO_V1 = AlgoConfig(
    name="v1",
    success_bonus=0.45,
    latency_divisor=2500.0,
    latency_cap=0.6,
    success_min_gain=None,
    failure_base=0.9,
    failure_step=0.35,
    failure_uses_offset=True,
    failure_cap=None,
    weight_min=-12.0,
    weight_max=12.0,
    success_floor=0.3,
    probe_every_requests=100,
    probe_interval_secs=30 * 60,
    probe_recovery_weight=0.4,
    min_positive_candidates=1,
    direct_initial_weight=1.0,
)

ALGO_V2 = AlgoConfig(
    name="v2",
    success_bonus=0.55,
    latency_divisor=9000.0,
    latency_cap=0.35,
    success_min_gain=0.08,
    failure_base=0.5,
    failure_step=0.18,
    failure_uses_offset=False,
    failure_cap=1.2,
    weight_min=-8.0,
    weight_max=8.0,
    success_floor=0.25,
    probe_every_requests=30,
    probe_interval_secs=5 * 60,
    probe_recovery_weight=0.55,
    min_positive_candidates=2,
    direct_initial_weight=0.7,
)

ALGO_BY_NAME = {"v1": ALGO_V1, "v2": ALGO_V2}


@dataclass
class RuntimeState:
    weight: float
    success_ema: float = 0.65
    latency_ema_ms: Optional[float] = None
    consecutive_failures: int = 0


@dataclass
class AttemptRow:
    event_id: int
    proxy_key: str
    occurred_at: str
    is_success: bool
    latency_ms: Optional[float]
    failure_kind: str
    is_probe: bool


@dataclass
class ProxyProfile:
    key: str
    observed_attempts: int
    success_rate: float
    success_latencies: List[float]
    failure_latencies: List[float]
    failure_kinds: List[str]


class ForwardProxyStateMachine:
    def __init__(
        self,
        algo: AlgoConfig,
        proxy_keys: Iterable[str],
        insert_direct: bool,
        seed_runtime: Optional[Dict[str, RuntimeState]] = None,
    ) -> None:
        self.algo = algo
        self.proxy_keys = sorted({key for key in proxy_keys if key})
        if insert_direct and DIRECT_KEY not in self.proxy_keys:
            self.proxy_keys.append(DIRECT_KEY)
        if not self.proxy_keys:
            self.proxy_keys.append(DIRECT_KEY)
        self.runtime: Dict[str, RuntimeState] = {
            key: RuntimeState(weight=self._default_weight(key)) for key in self.proxy_keys
        }
        if seed_runtime:
            for key, state in seed_runtime.items():
                self.runtime[key] = self._normalize_seed_state(state)
                if key not in self.proxy_keys:
                    self.proxy_keys.append(key)
        self.selection_counter = 0
        self.requests_since_probe = 0
        self.probe_in_flight = False
        self.last_probe_at = 0.0
        self.ensure_min_positive_candidates()

    def _default_weight(self, key: str) -> float:
        return self.algo.direct_initial_weight if key == DIRECT_KEY else 0.8

    def _valid_latency(self, latency_ms: Optional[float]) -> Optional[float]:
        if latency_ms is None:
            return None
        if isinstance(latency_ms, (float, int)) and math.isfinite(latency_ms) and latency_ms >= 0.0:
            return float(latency_ms)
        return None

    def _normalize_seed_state(self, state: RuntimeState) -> RuntimeState:
        weight = state.weight if math.isfinite(state.weight) else 0.0
        weight = max(self.algo.weight_min, min(self.algo.weight_max, weight))

        if math.isfinite(state.success_ema):
            success_ema = max(0.0, min(1.0, state.success_ema))
        else:
            success_ema = 0.65

        latency_ema_ms = self._valid_latency(state.latency_ema_ms)
        consecutive_failures = max(0, int(state.consecutive_failures))
        return RuntimeState(
            weight=weight,
            success_ema=success_ema,
            latency_ema_ms=latency_ema_ms,
            consecutive_failures=consecutive_failures,
        )

    def ensure_min_positive_candidates(self) -> None:
        positive_keys = [
            key
            for key, state in self.runtime.items()
            if state.weight > 0.0 and math.isfinite(state.weight)
        ]
        if len(positive_keys) >= self.algo.min_positive_candidates:
            return

        candidates = sorted(
            self.runtime.items(), key=lambda item: item[1].weight, reverse=True
        )
        for key, state in candidates:
            if len(positive_keys) >= self.algo.min_positive_candidates:
                break
            if state.weight > 0.0 and math.isfinite(state.weight):
                continue
            state.weight = self.algo.probe_recovery_weight
            if self.algo.name == "v2":
                state.consecutive_failures = 0
            positive_keys.append(key)

    def record_attempt(self, key: str, success: bool, latency_ms: Optional[float], is_probe: bool) -> None:
        if key not in self.runtime:
            self.runtime[key] = RuntimeState(weight=self._default_weight(key))
            if key not in self.proxy_keys:
                self.proxy_keys.append(key)
        state = self.runtime[key]

        latency = self._valid_latency(latency_ms)
        state.success_ema = state.success_ema * 0.9 + (0.1 if success else 0.0)
        if latency is not None:
            if state.latency_ema_ms is None:
                state.latency_ema_ms = latency
            else:
                state.latency_ema_ms = state.latency_ema_ms * 0.8 + latency * 0.2

        if success:
            state.consecutive_failures = 0
            latency_penalty = 0.0
            if state.latency_ema_ms is not None:
                latency_penalty = min(state.latency_ema_ms / self.algo.latency_divisor, self.algo.latency_cap)
            success_gain = self.algo.success_bonus - latency_penalty
            if self.algo.success_min_gain is not None:
                success_gain = max(self.algo.success_min_gain, success_gain)
            state.weight += success_gain
            if is_probe and state.weight <= 0.0:
                state.weight = self.algo.probe_recovery_weight
        else:
            state.consecutive_failures += 1
            if self.algo.failure_uses_offset:
                step_factor = max(0, state.consecutive_failures - 1)
            else:
                step_factor = state.consecutive_failures
            failure_penalty = self.algo.failure_base + step_factor * self.algo.failure_step
            if self.algo.failure_cap is not None:
                failure_penalty = min(self.algo.failure_cap, failure_penalty)
            state.weight -= failure_penalty

        state.weight = max(self.algo.weight_min, min(self.algo.weight_max, state.weight))

        if success and state.weight < self.algo.success_floor:
            state.weight = self.algo.success_floor

        self.ensure_min_positive_candidates()

    def positive_count(self) -> int:
        return sum(1 for state in self.runtime.values() if state.weight > 0.0 and math.isfinite(state.weight))

    def _select_weighted_key(self, rng: random.Random) -> str:
        candidates: List[Tuple[str, float]] = [
            (
                key,
                (
                    (state.weight**2) * max(state.success_ema**8, 0.01)
                    if self.algo.name == "v2"
                    else state.weight
                ),
            )
            for key, state in self.runtime.items()
            if state.weight > 0.0 and math.isfinite(state.weight)
        ]
        if not candidates:
            return DIRECT_KEY if DIRECT_KEY in self.runtime else self.proxy_keys[0]

        if self.algo.name == "v2" and len(candidates) > 3:
            candidates.sort(key=lambda item: item[1], reverse=True)
            candidates = candidates[:3]

        total_weight = sum(weight for _, weight in candidates)
        threshold = rng.random() * total_weight
        for key, weight in candidates:
            if threshold <= weight:
                return key
            threshold -= weight
        return candidates[-1][0]

    def select_proxy(self, rng: random.Random) -> str:
        self.selection_counter += 1
        self.requests_since_probe += 1
        self.ensure_min_positive_candidates()
        return self._select_weighted_key(rng)

    def should_probe_penalized(self, now_secs: float) -> bool:
        has_penalized = any(state.weight <= 0.0 for state in self.runtime.values())
        if not has_penalized or self.probe_in_flight:
            return False
        if self.requests_since_probe >= self.algo.probe_every_requests:
            return True
        return (now_secs - self.last_probe_at) >= self.algo.probe_interval_secs

    def start_probe(self, now_secs: float) -> Optional[str]:
        if not self.should_probe_penalized(now_secs):
            return None
        penalized = [
            (key, state.weight)
            for key, state in self.runtime.items()
            if state.weight <= 0.0
        ]
        if not penalized:
            return None
        penalized.sort(key=lambda item: item[1], reverse=True)
        self.probe_in_flight = True
        self.requests_since_probe = 0
        self.last_probe_at = now_secs
        return penalized[0][0]

    def finish_probe(self, now_secs: float) -> None:
        self.probe_in_flight = False
        self.last_probe_at = now_secs


def percentile(values: List[float], p: float) -> Optional[float]:
    if not values:
        return None
    if len(values) == 1:
        return float(values[0])
    ordered = sorted(values)
    rank = (len(ordered) - 1) * (p / 100.0)
    lower = int(math.floor(rank))
    upper = int(math.ceil(rank))
    if lower == upper:
        return float(ordered[lower])
    weight = rank - lower
    return float(ordered[lower] + (ordered[upper] - ordered[lower]) * weight)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Backtest forward proxy algorithms")
    parser.add_argument("--db", required=True, help="Path to sqlite database snapshot")
    parser.add_argument(
        "--algos",
        default="v1,v2",
        help="Comma-separated algorithms to evaluate (must include both v1 and v2)",
    )
    parser.add_argument(
        "--seeds",
        default="7,11,19,23,29",
        help="Comma-separated random seeds for stochastic simulation",
    )
    parser.add_argument(
        "--requests",
        type=int,
        default=50_000,
        help="Number of regular requests per seed in stochastic simulation",
    )
    parser.add_argument(
        "--output-prefix",
        default=None,
        help="Output prefix path without extension; default /tmp/forward-proxy-backtest-<timestamp>",
    )
    parser.add_argument(
        "--no-redact",
        action="store_true",
        help="Disable proxy-key redaction in output reports (not recommended)",
    )
    return parser.parse_args()


def repo_root_from_script() -> Path:
    return Path(__file__).resolve().parents[1]


def validate_inputs(args: argparse.Namespace) -> Tuple[Path, List[AlgoConfig], List[int], Path, Dict[str, bool]]:
    checks: Dict[str, bool] = {
        "db_exists": False,
        "db_readonly_uri_mode": False,
        "db_outside_repository": False,
        "output_in_tmp": False,
        "redaction_enabled": not args.no_redact,
    }

    db_path = Path(args.db).expanduser().resolve()
    if not db_path.exists() or not db_path.is_file():
        raise SystemExit(f"database not found: {db_path}")
    checks["db_exists"] = True

    repo_root = repo_root_from_script().resolve()
    try:
        db_path.relative_to(repo_root)
    except ValueError:
        checks["db_outside_repository"] = True
    else:
        raise SystemExit(
            f"refusing to use repository-local database path: {db_path} (must be outside {repo_root})"
        )

    algo_names = [part.strip().lower() for part in args.algos.split(",") if part.strip()]
    if not algo_names:
        raise SystemExit("--algos must include at least one algorithm")
    unknown_algos = [name for name in algo_names if name not in ALGO_BY_NAME]
    if unknown_algos:
        raise SystemExit(f"unknown algorithms: {', '.join(unknown_algos)}")
    algos = [ALGO_BY_NAME[name] for name in algo_names]

    seeds: List[int] = []
    for raw in args.seeds.split(","):
        raw = raw.strip()
        if not raw:
            continue
        try:
            seeds.append(int(raw))
        except ValueError as exc:
            raise SystemExit(f"invalid seed: {raw}") from exc
    if not seeds:
        raise SystemExit("--seeds must contain at least one integer")

    if args.requests <= 0:
        raise SystemExit("--requests must be > 0")

    timestamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%d-%H%M%S")
    if args.output_prefix:
        output_prefix = Path(args.output_prefix).expanduser().resolve()
    else:
        output_prefix = Path(f"/tmp/forward-proxy-backtest-{timestamp}")
    output_prefix = output_prefix.resolve()
    tmp_root = Path("/tmp").resolve()
    try:
        output_prefix.relative_to(tmp_root)
    except ValueError:
        checks["output_in_tmp"] = False
    else:
        checks["output_in_tmp"] = True

    return db_path, algos, seeds, output_prefix, checks


def open_readonly_connection(db_path: Path) -> Tuple[sqlite3.Connection, bool]:
    uri = f"file:{db_path}?mode=ro"
    conn = sqlite3.connect(uri, uri=True)
    conn.row_factory = sqlite3.Row
    return conn, "mode=ro" in uri.lower()


def load_attempts(conn: sqlite3.Connection) -> List[AttemptRow]:
    rows = conn.execute(
        """
        SELECT
            id,
            proxy_key,
            occurred_at,
            is_success,
            latency_ms,
            COALESCE(failure_kind, '') AS failure_kind,
            is_probe
        FROM forward_proxy_attempts
        ORDER BY occurred_at ASC, id ASC
        """
    ).fetchall()
    attempts: List[AttemptRow] = []
    for row in rows:
        key = str(row["proxy_key"] or "").strip()
        if not key:
            continue
        attempts.append(
            AttemptRow(
                event_id=int(row["id"]),
                proxy_key=key,
                occurred_at=str(row["occurred_at"]),
                is_success=bool(row["is_success"]),
                latency_ms=float(row["latency_ms"]) if row["latency_ms"] is not None else None,
                failure_kind=str(row["failure_kind"] or ""),
                is_probe=bool(row["is_probe"]),
            )
        )
    return attempts


def load_runtime_keys(conn: sqlite3.Connection) -> List[str]:
    rows = conn.execute(
        "SELECT proxy_key FROM forward_proxy_runtime WHERE proxy_key IS NOT NULL"
    ).fetchall()
    keys = []
    for row in rows:
        key = str(row["proxy_key"] or "").strip()
        if key:
            keys.append(key)
    return keys


def load_runtime_states(conn: sqlite3.Connection) -> Dict[str, RuntimeState]:
    rows = conn.execute(
        """
        SELECT proxy_key, weight, success_ema, latency_ema_ms, consecutive_failures
        FROM forward_proxy_runtime
        WHERE proxy_key IS NOT NULL
        """
    ).fetchall()
    states: Dict[str, RuntimeState] = {}
    for row in rows:
        key = str(row["proxy_key"] or "").strip()
        if not key:
            continue
        weight = float(row["weight"]) if row["weight"] is not None else 0.0
        success_ema = float(row["success_ema"]) if row["success_ema"] is not None else 0.65
        latency_ema_ms = float(row["latency_ema_ms"]) if row["latency_ema_ms"] is not None else None
        consecutive_failures = (
            int(row["consecutive_failures"]) if row["consecutive_failures"] is not None else 0
        )
        states[key] = RuntimeState(
            weight=weight,
            success_ema=max(0.0, min(1.0, success_ema)),
            latency_ema_ms=latency_ema_ms,
            consecutive_failures=max(0, consecutive_failures),
        )
    return states


def load_insert_direct(conn: sqlite3.Connection) -> bool:
    try:
        row = conn.execute(
            """
            SELECT insert_direct
            FROM forward_proxy_settings
            WHERE id = 1
            """
        ).fetchone()
    except sqlite3.OperationalError:
        return True
    if row is None:
        return True
    raw_value = row["insert_direct"]
    if raw_value is None:
        return True
    try:
        return bool(int(raw_value))
    except (TypeError, ValueError):
        return True


def redact_proxy(key: str, enable_redact: bool) -> str:
    if not enable_redact:
        return key
    digest = hashlib.sha256(key.encode("utf-8")).hexdigest()[:12]
    return f"proxy_{digest}"


def build_profiles(attempts: List[AttemptRow], proxy_keys: Iterable[str]) -> Dict[str, ProxyProfile]:
    grouped: Dict[str, List[AttemptRow]] = collections.defaultdict(list)
    for item in attempts:
        if item.is_probe:
            continue
        grouped[item.proxy_key].append(item)

    profiles: Dict[str, ProxyProfile] = {}
    for key in set(proxy_keys) | set(grouped.keys()):
        rows = grouped.get(key, [])
        if not rows:
            profiles[key] = ProxyProfile(
                key=key,
                observed_attempts=0,
                success_rate=0.65,
                success_latencies=[],
                failure_latencies=[],
                failure_kinds=["handshake_timeout"],
            )
            continue
        success_rows = [row for row in rows if row.is_success]
        failure_rows = [row for row in rows if not row.is_success]
        success_rate = len(success_rows) / len(rows)
        success_latencies = [
            row.latency_ms
            for row in success_rows
            if row.latency_ms is not None and math.isfinite(row.latency_ms) and row.latency_ms >= 0
        ]
        failure_latencies = [
            row.latency_ms
            for row in failure_rows
            if row.latency_ms is not None and math.isfinite(row.latency_ms) and row.latency_ms >= 0
        ]
        failure_kinds = [row.failure_kind or "handshake_timeout" for row in failure_rows]
        if not failure_kinds:
            failure_kinds = ["handshake_timeout"]
        profiles[key] = ProxyProfile(
            key=key,
            observed_attempts=len(rows),
            success_rate=success_rate,
            success_latencies=success_latencies,
            failure_latencies=failure_latencies,
            failure_kinds=failure_kinds,
        )
    return profiles


def sample_latency(profile: ProxyProfile, success: bool, rng: random.Random, fallback: List[float]) -> Optional[float]:
    source = profile.success_latencies if success else profile.failure_latencies
    if source:
        return float(rng.choice(source))
    if fallback:
        return float(rng.choice(fallback))
    return None


def trace_replay(
    attempts: List[AttemptRow], algo: AlgoConfig, proxy_keys: Iterable[str], insert_direct: bool
) -> Dict[str, object]:
    machine = ForwardProxyStateMachine(algo, proxy_keys, insert_direct=insert_direct)
    positive_counts: List[int] = []
    collapse_events = 0
    recovery_waits: List[int] = []
    pending_recovery: Dict[str, int] = {}

    for index, item in enumerate(attempts):
        machine.record_attempt(item.proxy_key, item.is_success, item.latency_ms, item.is_probe)
        positive = machine.positive_count()
        positive_counts.append(positive)
        if positive <= 1:
            collapse_events += 1

        runtime = machine.runtime[item.proxy_key]
        if not item.is_success and runtime.weight <= 0.0 and item.proxy_key not in pending_recovery:
            pending_recovery[item.proxy_key] = index

        to_remove = []
        for key, start_idx in pending_recovery.items():
            state = machine.runtime.get(key)
            if state is None:
                to_remove.append(key)
                continue
            if state.weight > 0.0:
                recovery_waits.append(index - start_idx)
                to_remove.append(key)
        for key in to_remove:
            pending_recovery.pop(key, None)

    return {
        "events": len(attempts),
        "positive_nodes_p50": float(statistics.median(positive_counts)) if positive_counts else 0.0,
        "positive_nodes_min": min(positive_counts) if positive_counts else 0,
        "single_node_collapse_ratio": (collapse_events / len(positive_counts)) if positive_counts else 0.0,
        "recovery_events_median": float(statistics.median(recovery_waits)) if recovery_waits else None,
        "recovery_events_p95": percentile(recovery_waits, 95.0) if recovery_waits else None,
        "recovery_samples": len(recovery_waits),
    }


def run_simulation(
    algo: AlgoConfig,
    profiles: Dict[str, ProxyProfile],
    seeds: List[int],
    requests: int,
    insert_direct: bool,
    seed_runtime: Optional[Dict[str, RuntimeState]],
) -> Dict[str, object]:
    proxy_keys = sorted(
        key for key, profile in profiles.items() if profile.observed_attempts > 0
    )
    if not proxy_keys:
        proxy_keys = sorted(profiles.keys())
    if insert_direct and DIRECT_KEY not in proxy_keys:
        proxy_keys.append(DIRECT_KEY)
    allowed_keys = set(proxy_keys)
    all_success_latencies = [
        latency
        for profile in profiles.values()
        for latency in profile.success_latencies
        if latency is not None
    ]
    all_failure_latencies = [
        latency
        for profile in profiles.values()
        for latency in profile.failure_latencies
        if latency is not None
    ]

    per_seed = []
    for seed in seeds:
        rng = random.Random(seed)
        runtime_seed = (
            {
                key: state
                for key, state in seed_runtime.items()
                if key in allowed_keys
            }
            if seed_runtime
            else None
        )
        enable_direct = insert_direct
        machine = ForwardProxyStateMachine(
            algo,
            proxy_keys,
            insert_direct=enable_direct,
            seed_runtime=runtime_seed,
        )
        selection_counter: collections.Counter[str] = collections.Counter()
        failure_kind_counter: collections.Counter[str] = collections.Counter()

        success_count = 0
        observed_latencies: List[float] = []
        now_secs = 0.0

        for _ in range(requests):
            key = machine.select_proxy(rng)
            selection_counter[key] += 1
            profile = profiles.get(key)
            if profile is None:
                profile = ProxyProfile(
                    key=key,
                    observed_attempts=0,
                    success_rate=0.65,
                    success_latencies=all_success_latencies,
                    failure_latencies=all_failure_latencies,
                    failure_kinds=["handshake_timeout"],
                )
            success = rng.random() < profile.success_rate
            latency = sample_latency(
                profile,
                success=success,
                rng=rng,
                fallback=all_success_latencies if success else all_failure_latencies,
            )

            if success:
                success_count += 1
            else:
                failure_kind = rng.choice(profile.failure_kinds) if profile.failure_kinds else "handshake_timeout"
                failure_kind_counter[failure_kind] += 1

            if latency is not None:
                observed_latencies.append(latency)
                now_secs += max(latency / 1000.0, 0.05)
            else:
                now_secs += 0.2

            machine.record_attempt(key, success, latency, is_probe=False)

            probe_key = machine.start_probe(now_secs)
            if probe_key is not None:
                probe_profile = profiles.get(probe_key, profile)
                probe_success = rng.random() < probe_profile.success_rate
                probe_latency = sample_latency(
                    probe_profile,
                    success=probe_success,
                    rng=rng,
                    fallback=all_success_latencies if probe_success else all_failure_latencies,
                )
                machine.record_attempt(probe_key, probe_success, probe_latency, is_probe=True)
                machine.finish_probe(now_secs)

        top1 = max(selection_counter.values()) if selection_counter else 0
        top1_key = selection_counter.most_common(1)[0][0] if selection_counter else ""
        per_seed.append(
            {
                "seed": seed,
                "success_rate": success_count / requests,
                "p95_latency_ms": percentile(observed_latencies, 95.0),
                "top1_share": top1 / requests,
                "top1_proxy_key": top1_key,
                "failure_kind_distribution": dict(failure_kind_counter),
            }
        )

    success_values = [item["success_rate"] for item in per_seed]
    p95_values = [item["p95_latency_ms"] for item in per_seed if item["p95_latency_ms"] is not None]
    top1_values = [item["top1_share"] for item in per_seed]

    return {
        "seeds": per_seed,
        "success_rate_mean": statistics.mean(success_values) if success_values else 0.0,
        "p95_latency_mean": statistics.mean(p95_values) if p95_values else None,
        "top1_share_mean": statistics.mean(top1_values) if top1_values else 0.0,
    }


def evaluate_acceptance(
    baseline: Dict[str, object],
    candidate: Dict[str, object],
    security_checks: Dict[str, bool],
) -> Dict[str, object]:
    failed: List[str] = []

    c_trace = candidate["trace_replay"]
    c_sim = candidate["simulation"]
    b_sim = baseline["simulation"]

    if c_trace["positive_nodes_p50"] < 2:
        failed.append("trace_replay.positive_nodes_p50 < 2")
    if c_trace["single_node_collapse_ratio"] > 0.35:
        failed.append("trace_replay.single_node_collapse_ratio > 0.35")
    if c_sim["top1_share_mean"] > 0.55:
        failed.append("simulation.top1_share_mean > 0.55")
    if c_sim["success_rate_mean"] < (b_sim["success_rate_mean"] - 0.002):
        failed.append("simulation.success_rate_mean below baseline-0.2%")

    baseline_p95 = b_sim["p95_latency_mean"]
    candidate_p95 = c_sim["p95_latency_mean"]
    if baseline_p95 is None or candidate_p95 is None:
        failed.append("simulation.p95_latency_mean missing")
    elif candidate_p95 > baseline_p95 * 1.10:
        failed.append("simulation.p95_latency_mean exceeds baseline*1.10")

    required_security = [
        "db_exists",
        "db_readonly_uri_mode",
        "db_outside_repository",
        "output_in_tmp",
        "redaction_enabled",
    ]
    for key in required_security:
        if not security_checks.get(key, False):
            failed.append(f"security_check_failed:{key}")

    return {
        "pass": len(failed) == 0,
        "failed_rules": failed,
    }


def write_markdown_report(
    output_md: Path,
    result: Dict[str, object],
    redact: bool,
) -> None:
    baseline = result.get("baseline", {})
    candidate = result.get("candidate", {})
    acceptance = result.get("acceptance", {})

    lines = [
        "# Forward Proxy Backtest Report",
        "",
        f"- Generated at: `{result.get('generated_at')}`",
        f"- Database: `{result.get('db_path')}`",
        f"- Redaction: `{'enabled' if redact else 'disabled'}`",
        f"- Acceptance: `{'PASS' if acceptance.get('pass') else 'FAIL'}`",
        "",
        "## Trace Replay",
        "",
        "| Metric | Baseline (v1) | Candidate (v2) |",
        "| --- | ---: | ---: |",
        f"| positive_nodes_p50 | {baseline.get('trace_replay', {}).get('positive_nodes_p50')} | {candidate.get('trace_replay', {}).get('positive_nodes_p50')} |",
        f"| single_node_collapse_ratio | {baseline.get('trace_replay', {}).get('single_node_collapse_ratio')} | {candidate.get('trace_replay', {}).get('single_node_collapse_ratio')} |",
        f"| recovery_events_median | {baseline.get('trace_replay', {}).get('recovery_events_median')} | {candidate.get('trace_replay', {}).get('recovery_events_median')} |",
        "",
        "## Stochastic Simulation",
        "",
        "| Metric | Baseline (v1) | Candidate (v2) |",
        "| --- | ---: | ---: |",
        f"| success_rate_mean | {baseline.get('simulation', {}).get('success_rate_mean')} | {candidate.get('simulation', {}).get('success_rate_mean')} |",
        f"| p95_latency_mean | {baseline.get('simulation', {}).get('p95_latency_mean')} | {candidate.get('simulation', {}).get('p95_latency_mean')} |",
        f"| top1_share_mean | {baseline.get('simulation', {}).get('top1_share_mean')} | {candidate.get('simulation', {}).get('top1_share_mean')} |",
        "",
    ]

    failed_rules = acceptance.get("failed_rules") or []
    if failed_rules:
        lines.append("## Failed Rules")
        lines.append("")
        for item in failed_rules:
            lines.append(f"- {item}")
        lines.append("")

    output_md.write_text("\n".join(lines), encoding="utf-8")


def sanitize_seed_output(simulation_result: Dict[str, object], redact: bool) -> None:
    for item in simulation_result.get("seeds", []):
        key = item.get("top1_proxy_key", "")
        item["top1_proxy_key"] = redact_proxy(key, enable_redact=redact)


def main() -> int:
    args = parse_args()
    db_path, algos, seeds, output_prefix, security_checks = validate_inputs(args)

    algo_by_name = {algo.name: algo for algo in algos}
    if "v1" not in algo_by_name or "v2" not in algo_by_name:
        raise SystemExit("--algos must include both v1 and v2 for acceptance evaluation")
    baseline_algo = algo_by_name["v1"]
    candidate_algo = algo_by_name["v2"]

    conn, readonly_uri_mode = open_readonly_connection(db_path)
    try:
        security_checks["db_readonly_uri_mode"] = readonly_uri_mode
        attempts = load_attempts(conn)
        runtime_keys = load_runtime_keys(conn)
        runtime_states = load_runtime_states(conn)
        insert_direct = load_insert_direct(conn)
    finally:
        conn.close()

    if not attempts:
        raise SystemExit("forward_proxy_attempts is empty; cannot run backtest")

    attempt_proxy_keys = sorted({item.proxy_key for item in attempts})
    trace_proxy_keys = sorted(set(attempt_proxy_keys) | ({DIRECT_KEY} if insert_direct else set()))
    profile_proxy_keys = sorted(set(runtime_keys) | set(attempt_proxy_keys))
    profiles = build_profiles(attempts, profile_proxy_keys)

    baseline = {
        "trace_replay": trace_replay(
            attempts,
            baseline_algo,
            trace_proxy_keys,
            insert_direct=insert_direct,
        ),
        "simulation": run_simulation(
            baseline_algo,
            profiles,
            seeds,
            args.requests,
            insert_direct,
            runtime_states,
        ),
    }
    candidate = {
        "trace_replay": trace_replay(
            attempts,
            candidate_algo,
            trace_proxy_keys,
            insert_direct=insert_direct,
        ),
        "simulation": run_simulation(
            candidate_algo,
            profiles,
            seeds,
            args.requests,
            insert_direct,
            runtime_states,
        ),
    }

    sanitize_seed_output(baseline["simulation"], redact=not args.no_redact)
    sanitize_seed_output(candidate["simulation"], redact=not args.no_redact)

    acceptance = evaluate_acceptance(baseline, candidate, security_checks)
    security_failed = [
        item
        for item in acceptance["failed_rules"]
        if item.startswith("security_check_failed:")
    ]
    if security_failed:
        print("acceptance_pass=False")
        print("failed_rules=")
        for item in acceptance["failed_rules"]:
            print(f"- {item}")
        return 1

    result = {
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "db_path": str(db_path),
        "config": {
            "algos": [algo.name for algo in algos],
            "seeds": seeds,
            "requests": args.requests,
            "redaction_enabled": not args.no_redact,
        },
        "security_checks": security_checks,
        "baseline": baseline,
        "candidate": candidate,
        "acceptance": acceptance,
    }

    output_json = output_prefix.with_suffix(".json")
    output_md = output_prefix.with_suffix(".md")
    output_json.write_text(json.dumps(result, ensure_ascii=True, indent=2), encoding="utf-8")
    write_markdown_report(output_md, result, redact=not args.no_redact)

    print(f"json_report={output_json}")
    print(f"markdown_report={output_md}")
    print(f"acceptance_pass={acceptance['pass']}")
    if acceptance["failed_rules"]:
        print("failed_rules=")
        for item in acceptance["failed_rules"]:
            print(f"- {item}")

    return 0 if acceptance["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
