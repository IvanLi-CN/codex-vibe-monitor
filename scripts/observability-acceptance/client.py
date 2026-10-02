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
import urllib.error
import urllib.parse
import urllib.request

ROOT=Path("/private")
POOL_TOKEN="pool-integration-fixture-token"
def request(base,path,method="GET",payload=None,token=None,basic=None):
    headers={"Accept":"application/json"}
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

def functional():
    scrape=(ROOT/"metrics-token").read_text().strip()
    read=(ROOT/"read-token").read_text().strip()
    viewer_token=(ROOT/"grafana-viewer-token").read_text().strip()
    for port in [9091,6772]:
        for token,status in [(None,401),("bad",401),(read,401),(scrape,200)]:
            assert request(f"http://app:{port}","/metrics",token=token)[0]==status,("scrape",port,status)
    for path,method in [("/api/system/performance","GET"),("/api/system/performance/health","GET"),("/api/system/performance/browser","POST")]:
        assert request("http://app:8080",path,method=method,payload={} if method=="POST" else None)[0]==410
    capabilities=ok("http://app:8080","/api/system/observability")
    assert not any("token" in key.lower() for key in capabilities)
    assert capabilities["grafanaConnectivity"]=="unknown"
    for uid in ["overview","proxy","sqlite","runtime","web"]:
        path="/api/dashboards/uid/cvm-"+uid
        for token in [None,"bad"]: assert request("https://entry:8443",path,token=token)[0] in (401,403)
        dashboard=ok("https://entry:8443",path,token=viewer_token)["dashboard"]
        assert dashboard["uid"]=="cvm-"+uid
        for panel in dashboard["panels"]:
            for target in panel.get("targets",[]):
                expression=target["expr"]
                for name,value in [("$__rate_interval","1m"),("$__range","30m"),("$service","codex-vibe-monitor"),("$environment","production"),("$instance","primary"),("$task_key",".*")]: expression=expression.replace(name,value)
                result=ok("https://entry:8443","/api/datasources/proxy/uid/cvm-prometheus/api/v1/query?"+urllib.parse.urlencode({"query":expression}),token=viewer_token)
                assert result["status"]=="success",expression
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
    assert request("http://app:8080","/api/system/observability/hotpath/reset",token=read)[0]==404
    # Shared read limiter is consumed deliberately only after all normal queries.
    statuses=[request("http://app:8080","/api/system/observability/hotpath/functions",token=read)[0] for _ in range(31)]
    assert 429 in statuses
    series=ok("http://prometheus:9090","/api/v1/query?query=count%28%7Bservice%3D%22codex-vibe-monitor%22%7D%29")["data"]["result"]
    assert series and float(series[0]["value"][1])<=5000,series
    up=ok("http://prometheus:9090","/api/v1/query?query=up")["data"]["result"]
    assert len(up)==2 and all(float(row["value"][1])==1 for row in up),up
    return {"httpsAuth":"passed","dashboards":5,"queries":"passed","reportBounds":"passed","series":float(series[0]["value"][1])}

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

parser=argparse.ArgumentParser();parser.add_argument("mode",choices=["seed","ready","viewer","functional","load"])
parser.add_argument("--seconds",type=int,default=60);parser.add_argument("--rate",type=int,default=20)
args=parser.parse_args()
result=load(args.seconds,args.rate) if args.mode=="load" else globals()[args.mode]()
print(json.dumps(result))
