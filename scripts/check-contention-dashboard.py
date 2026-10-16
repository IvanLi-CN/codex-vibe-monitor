#!/usr/bin/env python3
"""Validate provisioned Grafana PromQL against session-owned synthetic data."""

import argparse
import hashlib
import json
import math
from pathlib import Path
import urllib.parse
import urllib.request


SOURCE = Path(__file__).resolve().parent.parent
DASHBOARD = SOURCE / "ops/observability/grafana/dashboards/cvm-sqlite.json"
PROXY_DASHBOARD = SOURCE / "ops/observability/grafana/dashboards/cvm-proxy.json"


def substitute(expression, instance="fixture"):
    replacements = {"$__rate_interval": "1m", "$__range": "5m", "$compare_offset": "1d",
                    "$environment": "synthetic-preview", "$service": "codex-vibe-monitor",
                    "$instance": instance, "$endpoint": "responses|chat_completions", "$operation": ".*"}
    for key, value in replacements.items():
        expression = expression.replace(key, value)
    if "$" in expression:
        raise ValueError("unresolved dashboard variable")
    return expression


def fetch(url):
    with urllib.request.urlopen(url, timeout=30) as response:
        return json.load(response)


def evaluate(base, expression):
    response = fetch(base + "/api/datasources/proxy/uid/cvm-prometheus/api/v1/query?" +
                     urllib.parse.urlencode({"query": expression}))
    if response.get("status") != "success" or response.get("warnings"):
        raise AssertionError({"query": expression, "response": response})
    return response["data"]["result"]


def values(result):
    return [float(item["value"][1]) for item in result if "value" in item]


def validate(base):
    stored = json.loads(DASHBOARD.read_text())
    live = fetch(base + "/api/dashboards/uid/cvm-sqlite")["dashboard"]
    expected = {panel["id"]: panel for panel in stored["panels"]}
    actual = {panel["id"]: panel for panel in live["panels"]}
    # Grafana assigns its own database revision on provisioning; compare source content.
    for panel_id, panel in expected.items():
        assert actual[panel_id].get("targets") == panel.get("targets"), panel_id
    checked = 0
    for panel in stored["panels"]:
        for target in panel.get("targets", []):
            if "expr" not in target:
                continue
            result = evaluate(base, substitute(target["expr"]))
            assert all(math.isfinite(value) for value in values(result)), panel["title"]
            if 201 <= panel["id"] <= 236 and target["refId"] == "A":
                assert result, "normal fixture must populate " + panel["title"]
            checked += 1

    def query(panel_id, instance="fixture", target=0):
        return evaluate(base, substitute(expected[panel_id]["targets"][target]["expr"], instance))

    # Gate and distribution use exactly the same time window, including sparse data.
    for instance in ("fixture", "failures", "low-samples", "missing"):
        count = values(query(223, instance))
        low = instance == "low-samples"
        if instance == "missing":
            assert not query(222, instance) and not count
            assert not query(206, instance) and not query(227, instance)
            assert not query(236, instance), "missing samples must remain unknown"
        elif low:
            assert count and max(count) < 100
            assert not query(222, instance), "low-sample p95 must not be published"
            assert values(query(236, instance)) == [0]
        else:
            assert query(222, instance), "normal success p95 requires at least 100 samples"
    assert all(0 < value < .5 for value in values(query(222)))
    budgets = query(203)
    assert {"write_hold", "transaction"} <= {item["metric"]["phase"] for item in query(201)}, "permit hold and transaction must be independently visible"
    assert {item["metric"]["phase"] for item in budgets} == {">250ms", ">1s", ">10s"}
    assert all(value >= 0 for value in values(budgets))
    assert max(values(query(206, "failures"))) > 1000
    assert max(values(query(214, "failures"))) > 180
    assert values(query(214)) and set(values(query(214))) == {0}
    assert any(item["metric"].get("outcome") == "unavailable" and
               float(item["value"][1]) > 0 for item in query(221, "failures"))
    assert not query(234, target=1), "unseeded historic window must remain unknown"

    # The preview converts hotpath histograms to NHCB; validate native and classic separately.
    selector = '{service="codex-vibe-monitor",environment="synthetic-preview",instance="fixture"}'
    classic = evaluate(base, "histogram_quantile(.95,sum by(le,query)(rate(hotpath_sql_duration_seconds_bucket" + selector + "[1m])))")
    native = evaluate(base, "histogram_quantile(.95,sum by(query)(rate(hotpath_sql_duration_seconds" + selector + "[1m])))")
    classic_values = {item["metric"]["query"]: float(item["value"][1]) for item in classic}
    native_values = {item["metric"]["query"]: float(item["value"][1]) for item in native}
    assert classic_values and classic_values.keys() == native_values.keys()
    for key in classic_values:
        assert math.isclose(classic_values[key], native_values[key], rel_tol=1e-6)

    proxy_stored = json.loads(PROXY_DASHBOARD.read_text())
    proxy_live = fetch(base + "/api/dashboards/uid/cvm-proxy")["dashboard"]
    proxy_actual = {panel["id"]: panel for panel in proxy_live["panels"]}
    proxy_checked = 0
    for panel in proxy_stored["panels"]:
        if panel["id"] not in (112, 113, 114):
            continue
        assert proxy_actual[panel["id"]]["targets"] == panel["targets"], panel["id"]
        for target in panel["targets"]:
            result = evaluate(base, substitute(target["expr"]))
            assert result and all(math.isfinite(value) and value >= 0 for value in values(result)), panel["title"]
            proxy_checked += 1
    return {"dashboard_uid": live["uid"], "grafana_database_revision": live["version"],
            "dashboard_source_version": stored["version"],
            "dashboard_source_sha256": hashlib.sha256(DASHBOARD.read_bytes()).hexdigest(),
            "queries_checked": checked, "proxy_queries_checked": proxy_checked,
            "proxy_dashboard_source_sha256": hashlib.sha256(PROXY_DASHBOARD.read_bytes()).hexdigest(),
            "scenarios": ["normal", "failures", "low-samples", "missing"],
            "native_classic_agree": True, "history_without_samples": "unknown"}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--grafana-url", required=True, help="session-owned synthetic preview only")
    arguments = parser.parse_args()
    print(json.dumps(validate(arguments.grafana_url.rstrip("/")), indent=2))
