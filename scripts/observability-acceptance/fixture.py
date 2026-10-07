#!/usr/bin/env python3
"""Mock-only integration peer and controlled HTTPS entry for the testbox run."""
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import re
import ssl
import sys
import time
import os
shared = Path(__file__).resolve().parent.parent.parent / "ops/observability"
sys.path.insert(0, str(shared) if shared.is_dir() else "/observability")
from tempo_access import authorized, query_route, TENANT

class MockUpstream(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *_): pass
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        payload = json.loads(self.rfile.read(length))
        time.sleep(0.02)
        result = {"id":"resp_fixture","object":"response","model":"gpt-5","status":"completed","output":[],"usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}
        if payload.get("stream"):
            body = ("event: response.completed\ndata: " + json.dumps({"type":"response.completed","response":result}) + "\n\n").encode()
            content_type = "text/event-stream"
        else:
            body = json.dumps(result).encode(); content_type = "application/json"
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers(); self.wfile.write(body)

class HttpsEntry(BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def deny(self, status):
        body=b'{"error":"entry_access_denied"}'
        self.send_response(status); self.send_header("Content-Type","application/json")
        self.send_header("Content-Length",str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_GET(self):
        path = self.path.split("?",1)[0]
        grafana = re.fullmatch(r"/api/datasources/proxy/uid/cvm-prometheus/api/v1/(query|query_range)",path) or re.fullmatch(r"/api/dashboards/uid/cvm-(overview|proxy|proxy-cases|sqlite|runtime|web)",path)
        if path.startswith("/tempo/"):
            if not authorized(self.headers.get("Authorization"), Path("/private/tempo-query-token").read_text().strip()):
                self.deny(401); return
            try: route = query_route(self.path.removeprefix("/tempo"), fixed_cases=False)
            except (ValueError, TypeError): self.deny(400); return
            self.tempo("GET", 3200, route); return
        if re.fullmatch(r"/api/datasources/proxy/uid/cvm-tempo/api/(search|v2/traces/[a-f0-9]{32})", path):
            try: query_route(self.path.removeprefix("/api/datasources/proxy/uid/cvm-tempo"))
            except (ValueError, TypeError): self.deny(400); return
            grafana = True
        app = re.fullmatch(r"/api/system/observability/hotpath/(server|sql|functions)",path)
        if not grafana and not app:
            self.deny(401); return
        connection=http.client.HTTPConnection("grafana" if grafana else "app",3000 if grafana else 8080,timeout=5)
        connection.request("GET",self.path,headers={"Authorization":self.headers.get("Authorization",""),"Accept":"application/json"})
        response=connection.getresponse(); body=response.read(1024*1024+1)
        if len(body)>1024*1024: self.deny(502); connection.close(); return
        self.send_response(response.status); self.send_header("Content-Type",response.getheader("Content-Type","application/json"))
        self.send_header("Content-Length",str(len(body))); self.end_headers(); self.wfile.write(body); connection.close()
    def tempo(self, method, port, route, body=None, content_type="application/x-protobuf"):
        connection = http.client.HTTPConnection("tempo", port, timeout=5)
        accept = self.headers.get("Accept", "application/json")
        if accept not in ("application/protobuf", "application/json"): accept = "application/json"
        connection.request(method, route, body=body, headers={"X-Scope-OrgID":TENANT, "Accept":accept, "Content-Type":content_type})
        response = connection.getresponse(); content = response.read(1024*1024+1)
        if len(content)>1024*1024: connection.close(); self.deny(502); return
        self.send_response(response.status); self.send_header("Content-Type",response.getheader("Content-Type","application/json"))
        self.send_header("Content-Length",str(len(content))); self.end_headers(); self.wfile.write(content); connection.close()
    def do_POST(self):
        if self.path != "/v1/traces": self.deny(405); return
        if not authorized(self.headers.get("Authorization"), Path("/private/tempo-ingest-token").read_text().strip()):
            self.deny(401); return
        try: length = int(self.headers.get("Content-Length", "-1"))
        except ValueError: self.deny(400); return
        if not 0 <= length <= 1024*1024 or self.headers.get("Transfer-Encoding"):
            self.deny(413); return
        self.connection.settimeout(2)
        content_type = self.headers.get("Content-Type", "")
        if content_type not in {"application/x-protobuf", "application/json"}: self.deny(415); return
        self.tempo("POST",4318,"/v1/traces",self.rfile.read(length),content_type)
    def do_PUT(self): self.deny(405)
    def do_DELETE(self): self.deny(405)

if sys.argv[1]=="upstream":
    ThreadingHTTPServer(("0.0.0.0",18080),MockUpstream).serve_forever()
elif sys.argv[1]=="https":
    server=ThreadingHTTPServer(("0.0.0.0",8443),HttpsEntry)
    context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain("/private/tls.crt","/private/tls.key")
    server.socket=context.wrap_socket(server.socket,server_side=True); server.serve_forever()
else: raise SystemExit("unknown fixture mode")
