#!/usr/bin/env python3
"""Run observable assertions inside the isolated Compose network; emit no secrets."""
import argparse
import base64
from concurrent.futures import ThreadPoolExecutor
import http.client
import json
import math
from pathlib import Path
import ssl
import socket
import threading
import time
import re
import secrets
import sys
shared = Path(__file__).resolve().parent.parent.parent / "ops/observability"
sys.path.insert(0, str(shared) if shared.is_dir() else "/observability")
from tempo_access import case_query, CASE_PREDICATES
import urllib.error
import urllib.parse
import urllib.request

ROOT=Path("/private")
POOL_TOKEN="pool-integration-fixture-token"
def request(base,path,method="GET",payload=None,token=None,basic=None,extra_headers=None):
    headers={"Accept":"application/json"}
    headers.update(extra_headers or {})
    if token: headers["Authorization"]="Bearer "+token
    if basic: headers["Authorization"]="Basic "+base64.b64encode(basic.encode()).decode()
    data=None if payload is None else json.dumps(payload).encode()
    if data: headers["Content-Type"]="application/json"
    context=ssl.create_default_context(cafile=str(ROOT/"tls.crt")) if base.startswith("https:") else None
    req=urllib.request.Request(base+path,data=data,headers=headers,method=method)
    try:
        with urllib.request.urlopen(req,timeout=10,context=context) as response:
            return response.status,response.read(1024*1024+1)
    except urllib.error.HTTPError as error: return error.code,error.read(1024*1024+1)

def ok(base,path,**kwargs):
    status,body=request(base,path,**kwargs)
    assert status in (200,201),(path,status)
    return json.loads(body)

def seed():
    ok("http://app:8080","/api/pool/routing-settings",method="PUT",payload={"apiKey":POOL_TOKEN})
    ok("http://app:8080","/api/pool/upstream-accounts/api-keys",method="POST",payload={"displayName":"Observability fixture","apiKey":"synthetic-upstream-token","boundProxyKeys":["__direct__"],"upstreamBaseUrl":"http://mock-upstream:18080/"})
    return {"seed":"passed"}

def viewer():
    basic="admin:"+(ROOT/"grafana-admin-password").read_text().strip()
    account=ok("http://grafana:3000","/api/serviceaccounts",method="POST",payload={"name":"cvm-fixture-agent","role":"Viewer"},basic=basic)
    token=ok("http://grafana:3000",f'/api/serviceaccounts/{account["id"]}/tokens',method="POST",payload={"name":"acceptance"},basic=basic)["key"]
    (ROOT/"grafana-viewer-token").write_text(token); (ROOT/"grafana-viewer-token").chmod(0o600)
    return {"viewer":"created"}

def ready():
    assert ok("http://grafana:3000", "/api/health")["database"] == "ok"
    return {"grafana":"ready"}

def browser_batch():
    events=[{"page":"records" if kind=="unsupported" else "dashboard","device":"desktop","kind":kind,"valueSeconds":0.02} for kind in ["data_ready","update_to_paint","api_request","long_task","sse","unsupported","hidden","dropped"]]
    events[4]["outcome"]="normal"
    payload={"events":events}
    assert len(json.dumps(payload).encode())<=2048
    assert request("http://app:8080","/api/system/observability/browser",method="POST",payload=payload,extra_headers={"Origin":"http://app:8080","Sec-Fetch-Site":"same-origin"})[0]==204

def browser_seed():
    token=(ROOT/"grafana-viewer-token").read_text().strip()
    dashboard=ok("https://entry:8443","/api/dashboards/uid/cvm-web",token=token)["dashboard"]
    data_panels=[panel for panel in dashboard["panels"] if panel.get("type")!="row"]
    assert data_panels and all(panel["fieldConfig"]["defaults"]["noValue"]=="Unknown" for panel in data_panels)
    missing=ok("http://prometheus:9090","/api/v1/query?"+urllib.parse.urlencode({"query":"cvm_browser_data_ready_seconds_count"}))["data"]["result"]
    assert missing==[],"missing browser collection was represented as a measured zero"
    browser_batch()
    (ROOT/"browser-seeded-at").write_text(str(time.time()))
    return {"browserMissing":"unknown","browserSeed":"accepted"}

def functional():
    scrape=(ROOT/"metrics-token").read_text().strip()
    read=(ROOT/"read-token").read_text().strip()
    viewer_token=(ROOT/"grafana-viewer-token").read_text().strip()
    for port in [9091,6772]:
        for token,status in [(None,401),("bad",401),(read,401),(scrape,200)]:
            assert request(f"http://app:{port}","/metrics",token=token)[0]==status,("scrape",port,status)
    for path in ["/api/system/performance","/api/system/performance/health","/api/system/performance/browser"]:
        for method in ["GET","POST","PUT","PATCH","DELETE","OPTIONS","HEAD"]:
            assert request("http://app:8080",path,method=method,payload={} if method in ["POST","PUT","PATCH"] else None)[0]==410,(path,method)
    capabilities=ok("http://app:8080","/api/system/observability")
    assert not any("token" in key.lower() for key in capabilities)
    assert capabilities["grafanaConnectivity"]=="unknown"
    assert {"enabled", "state", "grafanaPublicUrl", "grafanaConnectivity", "hotpath", "dashboards", "datasourceUid", "variables"} <= set(capabilities)
    assert set(capabilities["tracing"]) == {"enabled", "state", "datasourceUid", "caseDashboardUid"}
    assert capabilities["tracing"]["enabled"] is True
    assert capabilities["tracing"]["datasourceUid"] == "cvm-tempo"
    assert capabilities["tracing"]["caseDashboardUid"] == "cvm-proxy-cases"
    # A second bounded batch after one minute creates real counter/histogram deltas.
    time.sleep(max(0,60-(time.time()-float((ROOT/"browser-seeded-at").read_text()))))
    browser_batch()
    deadline=time.monotonic()+45
    while True:
        collected=ok("http://prometheus:9090","/api/v1/query?"+urllib.parse.urlencode({"query":"sum(cvm_browser_data_ready_seconds_count)"}))["data"]["result"]
        if collected and float(collected[0]["value"][1])>=2: break
        assert time.monotonic()<deadline,"browser observations were not scraped"
        time.sleep(2)
    web_samples=0
    for uid in ["overview","proxy","sqlite","runtime","web"]:
        path="/api/dashboards/uid/cvm-"+uid
        for token in [None,"bad"]: assert request("https://entry:8443",path,token=token)[0] in (401,403)
        dashboard=ok("https://entry:8443",path,token=viewer_token)["dashboard"]
        assert dashboard["uid"]=="cvm-"+uid
        for panel in dashboard["panels"]:
            for target in panel.get("targets",[]):
                expression=target["expr"]
                for name,value in [("$__rate_interval","1m"),("$__range_s","1800"),("$__range","30m"),("$service","codex-vibe-monitor"),("$environment","production"),("$instance","primary"),("$endpoint","responses|chat_completions"),("$phase","request_read"),("$resource","sqlite_coordinator"),("$task_key",".*"),("$lock_label",".*"),("$page",".*"),("$device",".*")]: expression=expression.replace(name,value)
                result=ok("https://entry:8443","/api/datasources/proxy/uid/cvm-prometheus/api/v1/query?"+urllib.parse.urlencode({"query":expression}),token=viewer_token)
                assert result["status"]=="success",expression
                if uid=="web":
                    samples=result["data"]["result"]
                    assert all(math.isfinite(float(row["value"][1])) for row in samples),(panel["title"],expression,samples)
                    web_samples+=len(samples)
    assert web_samples>0,"browser dashboard returned no observed series"
    for base in ["https://entry:8443","http://grafana:3000"]:
        assert request(base,"/api/dashboards/db",method="POST",payload={"dashboard":{"title":"forbidden"}},token=viewer_token)[0] in (403,405)
    assert request("https://entry:8443","/")[0]==401
    for report in ["server","sql","functions"]:
        path="/api/system/observability/hotpath/"+report
        for token in [None,"bad",scrape]: assert request("https://entry:8443",path,token=token)[0]==401
        status,body=request("https://entry:8443",path,token=read)
        assert status==200,(report,status)
        result=json.loads(body); assert len(result["rows"])<=100 and len(body)<=1024*1024
        assert result["functionSamplingRate"]==0.1
        assert result["rows"],("empty profiler report after representative traffic",report)
    # Unsupported collection must not synthesize a long-task sample for that page.
    unsupported=ok("http://prometheus:9090","/api/v1/query?"+urllib.parse.urlencode({"query":'cvm_browser_long_task_duration_seconds_count{page="records"}'}))["data"]["result"]
    assert unsupported==[],"unsupported browser observer fabricated samples"
    # Exercised proxy, CPU and SQL panels must also contain finite observations.
    observations={}
    for signal,expression in {
        "cpuRate":"sum(rate(cvm_process_cpu_seconds_total[1m]))",
        "threadCount":"sum(cvm_process_threads)",
        "proxyCompletions":"sum(cvm_proxy_invocations_total{outcome=\"success\"})",
        "upstreamAttempts":"sum(cvm_proxy_upstream_attempts_total)",
        "bodyP95":"histogram_quantile(0.95,sum by (le)(rate(cvm_http_body_duration_seconds_bucket{route=\"/v1/responses\"}[1m])))",
        "requestDiagnostics":"sum(cvm_request_response_duration_seconds_count{endpoint=\"responses\"})",
        "requestWaitDenominator":"sum(cvm_request_local_wait_seconds_count{endpoint=\"responses\"})",
        "requestPhaseEvents":"sum(cvm_request_stage_seconds_count)",
        "poolP95":"histogram_quantile(0.95,sum by (le)(rate(cvm_sqlite_pool_acquire_duration_seconds_bucket[1m])))",
        "sqlP95":"histogram_quantile(0.95,sum(rate(hotpath_sql_duration_seconds[1m])))",
        "sqlSamples":"sum(histogram_count(increase(hotpath_sql_duration_seconds[1m])))",
        "functionSamples":"sum(histogram_count(increase(hotpath_function_duration_seconds[1m])))",
    }.items():
        result=ok("https://entry:8443","/api/datasources/proxy/uid/cvm-prometheus/api/v1/query?"+urllib.parse.urlencode({"query":expression}),token=viewer_token)["data"]["result"]
        assert len(result)==1,("missing exercised metric",signal)
        value=float(result[0]["value"][1])
        assert math.isfinite(value) and value>0,("invalid exercised metric",signal,value)
        observations[signal]=value
    assert request("http://app:8080","/api/system/observability/hotpath/reset",token=read)[0]==404
    # Shared read limiter is consumed deliberately only after all normal queries.
    statuses=[request("http://app:8080","/api/system/observability/hotpath/functions",token=read)[0] for _ in range(31)]
    assert 429 in statuses
    series=ok("http://prometheus:9090","/api/v1/query?query=count%28%7Bservice%3D%22codex-vibe-monitor%22%7D%29")["data"]["result"]
    assert series and float(series[0]["value"][1])<=5000,series
    up=ok("http://prometheus:9090","/api/v1/query?query=up")["data"]["result"]
    assert len(up)==2 and all(float(row["value"][1])==1 for row in up),up
    return {"httpsAuth":"passed","dashboards":5,"queries":"passed","reportBounds":"passed","series":float(series[0]["value"][1]),"observations":observations}

def load(seconds,rate):
    def once(sequence):
        started=time.perf_counter()
        status,_=request("http://app:8080","/v1/responses",method="POST",payload={"model":"gpt-5","input":"fixture-"+str(sequence),"stream":sequence%2==0},token=POOL_TOKEN)
        return status,time.perf_counter()-started
    # Keep exactly one real dashboard subscription in both A/B states. Read the first
    # frame before starting the window, then drain updates until the window ends.
    topics=json.dumps([{"topic":"dashboard.activity.current","params":{"range":"today","timeZone":"UTC","includeAccounts":"true","includeRecent":"true","recentLimit":"16"}}],separators=(",",":"))
    encoded=base64.urlsafe_b64encode(topics.encode()).decode().rstrip("=")
    url="http://app:8080/events?"+urllib.parse.urlencode({"topics":encoded})
    connection=http.client.HTTPConnection("app",8080,timeout=30)
    connection.request("GET",url.removeprefix("http://app:8080"))
    subscription=connection.getresponse()
    assert subscription.status==200 and "text/event-stream" in subscription.getheader("Content-Type",""),("dashboard subscription",subscription.status,subscription.getheader("Content-Type",""))
    def frame():
        data=[];size=0
        while True:
            line=subscription.readline(1024*1024+1)
            if not line: raise RuntimeError("dashboard subscription closed during load")
            size+=len(line)
            if size>1024*1024: raise RuntimeError("dashboard subscription frame exceeds 1 MiB")
            if line in (b"\n",b"\r\n"):
                if not data: return None
                event=json.loads(b"\n".join(data))
                assert event["type"] in ("snapshot","live","replay"),"dashboard subscription unavailable"
                assert event["topic"]["topic"]=="dashboard.activity.current" and isinstance(event["payload"],dict),"invalid dashboard subscription data"
                return event
            if line.startswith(b"data:"): data.append(line[5:].lstrip().rstrip(b"\r\n"))
    while frame() is None: pass
    stop=threading.Event(); failures=[]
    def drain():
        try:
            while not stop.is_set():
                frame()
        except (OSError,RuntimeError,AssertionError,ValueError,KeyError):
            if not stop.is_set(): failures.append("dashboard subscription failed during load")
    reader=threading.Thread(target=drain,daemon=True);reader.start()
    started=time.perf_counter(); work=[]
    try:
        with ThreadPoolExecutor(max_workers=16) as executor:
            for sequence in range(seconds*rate):
                delay=started+sequence/rate-time.perf_counter()
                if delay>0: time.sleep(delay)
                work.append(executor.submit(once,sequence))
            results=[future.result() for future in work]
    finally:
        stop.set()
        if connection.sock: connection.sock.shutdown(socket.SHUT_RDWR)
        reader.join(timeout=2);subscription.close();connection.close()
    assert not reader.is_alive(),"dashboard subscriber did not stop"
    assert not failures,failures
    durations=sorted(item[1] for item in results)
    assert all(item[0]==200 for item in results),{"statuses":{str(status):sum(item[0]==status for item in results) for status,_ in results}}
    return {"offered":seconds*rate,"completed":len(results),"dashboardSubscriptions":1,"durationSeconds":time.perf_counter()-started,"p95Seconds":durations[math.ceil(len(durations)*0.95)-1]}

def trace_stats():
    enabled=ok("http://app:8080","/api/system/observability")["tracing"]
    status,body=request("http://app:9091","/metrics",token=(ROOT/"metrics-token").read_text().strip())
    assert status==200
    def counter(name):
        return sum(float(value) for value in re.findall(r"^"+name+r"(?:\{[^}]*\})? (\d+(?:\.\d+)?)$",body.decode(),re.M))
    return {"enabled":enabled["enabled"] and enabled["state"]=="enabled", "exportedSpans":counter("cvm_trace_exported_spans_total"), "failedSpans":counter("cvm_trace_export_failed_spans_total"), "droppedSpans":counter("cvm_trace_queue_dropped_spans_total")+counter("cvm_diagnostic_dropped_total")+counter("cvm_metric_series_dropped_total"), "searchableTrace": bool(ok("https://entry:8443", "/tempo/api/search?"+urllib.parse.urlencode({"q":'{ resource.service.name = "codex-vibe-monitor" && span.cvm.record = "response" && span.cvm.status_class = "2xx" }', "start":int(time.time())-300,"end":int(time.time()),"limit":1}), token=(ROOT/"tempo-query-token").read_text().strip()).get("traces"))}

def trace_cases():
    query_token=(ROOT/"tempo-query-token").read_text().strip()
    ingest_token=(ROOT/"tempo-ingest-token").read_text().strip()
    def search(category, environment=None, instance=None):
        now=int(time.time())
        return "/tempo/api/search?"+urllib.parse.urlencode({"q":case_query(category, environment=environment, instance=instance),"start":now-300,"end":now,"limit":3})
    for token in [None, "invalid", ingest_token]:
        assert request("https://entry:8443",search("normal"),token=token)[0]==401
    for token in [None, "invalid", query_token]:
        assert request("https://entry:8443","/v1/traces",method="POST",payload={},token=token)[0]==401
    assert trace_stats()["enabled"], "application trace startup degraded"
    assert request("http://app:8080", "/v1/responses", method="POST", payload={"input":"synthetic rejection"})[0] == 401
    deadline=time.monotonic()+15
    while True:
        rejected=ok("https://entry:8443","/tempo/api/search?"+urllib.parse.urlencode({"q":'{ resource.service.name = "codex-vibe-monitor" && span.cvm.record = "response" && span.cvm.status_class = "4xx" }',"start":int(time.time())-300,"end":int(time.time()),"limit":3}),token=query_token).get("traces",[])
        if rejected: break
        assert time.monotonic()<deadline, "pre-auth rejection trace is missing"
        time.sleep(1)
    rejection=ok("https://entry:8443","/tempo/api/v2/traces/"+rejected[0]["traceID"],token=query_token)
    def response_roots(node):
        if isinstance(node, dict):
            if node.get("name") == "cvm.proxy.response": yield node
            for child in node.values(): yield from response_roots(child)
        elif isinstance(node, list):
            for child in node: yield from response_roots(child)
    roots=list(response_roots(rejection))
    assert len(roots)==1
    attributes={a["key"]:a["value"] for a in roots[0]["attributes"]}
    assert int(attributes["cvm.attempts"]["intValue"]) == 0
    assert attributes["cvm.ttft_state"]["stringValue"] == "not_applicable"
    assert "invoke_id" not in json.dumps(rejection)
    # Search actual application records before injecting classified synthetic fixtures.
    deadline=time.monotonic()+20
    while True:
        actual=ok("https://entry:8443","/tempo/api/search?"+urllib.parse.urlencode({"q":'{ resource.service.name = "codex-vibe-monitor" && span.cvm.record = "response" && span.cvm.status_class = "2xx" }',"start":int(time.time())-300,"end":int(time.time()),"limit":3}),token=query_token).get("traces",[])
        if actual: break
        assert time.monotonic()<deadline, "application traces are not searchable"
        time.sleep(1)
    actual_id=actual[0]["traceID"]
    # Some Tempo builds omit a leading zero when serializing a trace ID. Keep
    # the gateway's fixed 32-hex contract while normalizing that fixture-only
    # representation before the detail lookup.
    assert re.fullmatch(r"[a-f0-9]{31,32}", actual_id)
    actual_id=actual_id.rjust(32, "0")
    deadline=time.monotonic()+15
    while True:
        actual_trace=ok("https://entry:8443","/tempo/api/v2/traces/"+actual_id,token=query_token,extra_headers={"X-Scope-OrgID":"other|cvm"})
        text=json.dumps(actual_trace)
        if "cvm.terminal.enqueue_to_commit" in text: break
        assert time.monotonic()<deadline, "late persistence export did not arrive"
        time.sleep(1)
    assert "cvm.proxy.response" in text
    assert "cvm.terminal.enqueue_to_commit" in text, "late persistence correlation missing"
    assert not any(secret in text for secret in [POOL_TOKEN,"synthetic-upstream-token","resp_fixture","invoke_id","prompt_cache_key"])
    # These model offline scenarios to test classification, not application timing or overhead.
    now=time.time_ns(); ids={}; spans=[]
    def attr(key,value):
        kind="boolValue" if isinstance(value,bool) else "intValue" if isinstance(value,int) else "stringValue"
        return {"key":key,"value":{kind:str(value) if kind=="intValue" else value}}
    for category in CASE_PREDICATES:
        trace_id=secrets.token_hex(16); ids[category]=trace_id
        flags={"cvm.record":"response","cvm.endpoint":"responses","cvm.fixture":True,"cvm.outcome":"error" if category=="error" else "complete","cvm.status_class":"5xx" if category=="error" else "2xx","cvm.normal_candidate":category=="normal","cvm.slow":category=="slow","cvm.high_wait":category=="wait","cvm.retry":category=="retry"}
        duration=31_000_000_000 if category=="slow" else 300_000_000
        spans.append({"traceId":trace_id,"spanId":secrets.token_hex(8),"name":"cvm.proxy.response","kind":2,"startTimeUnixNano":str(now-duration),"endTimeUnixNano":str(now),"attributes":[attr(k,v) for k,v in flags.items()]})
    # Keep injected classification cases distinct from real application records.
    # A valid limit=3 query can otherwise return other matching application traces.
    payload={"resourceSpans":[{"resource":{"attributes":[attr("service.name","codex-vibe-monitor"),attr("deployment.environment.name","classification-fixture"),attr("service.instance.id","fixture")]},"scopeSpans":[{"scope":{"name":"cvm.synthetic.classified"},"spans":spans}]}]}
    assert request("https://entry:8443","/v1/traces",method="POST",payload=payload,token=ingest_token,extra_headers={"X-Scope-OrgID":"other"})[0]==200
    for category, trace_id in ids.items():
        deadline=time.monotonic()+15
        while True:
            rows=ok("https://entry:8443",search(category, "classification-fixture", "fixture"),token=query_token).get("traces",[])
            if any(row["traceID"]==trace_id for row in rows): break
            assert time.monotonic()<deadline, "classified trace not searchable: "+category
            time.sleep(1)
        expanded=ok("https://entry:8443","/tempo/api/v2/traces/"+trace_id,token=query_token)
        assert "cvm.proxy.response" in json.dumps(expanded)
    return {"applicationTrace":"search-and-late-persistence-verified", "preAuthRejection":"no-attempt-or-business-identity", "classifiedFixtures":list(ids), "tenant":"credential-assigned", "unauthorizedRoles":"rejected"}

if __name__ == "__main__":
    parser=argparse.ArgumentParser();parser.add_argument("mode",choices=["seed","ready","viewer","browser_seed","functional","load","trace_stats","trace_cases"])
    parser.add_argument("--seconds",type=int,default=60);parser.add_argument("--rate",type=int,default=20)
    args=parser.parse_args()
    result=load(args.seconds,args.rate) if args.mode=="load" else globals()[args.mode]()
    print(json.dumps(result))
