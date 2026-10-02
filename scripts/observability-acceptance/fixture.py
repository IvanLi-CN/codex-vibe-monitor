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
        grafana = re.fullmatch(r"/api/datasources/proxy/uid/cvm-prometheus/api/v1/(query|query_range)",path) or re.fullmatch(r"/api/dashboards/uid/cvm-(overview|proxy|sqlite|runtime|web)",path)
        app = re.fullmatch(r"/api/system/observability/hotpath/(server|sql|functions)",path)
        if not grafana and not app:
            self.deny(401); return
        connection=http.client.HTTPConnection("grafana" if grafana else "app",3000 if grafana else 8080,timeout=5)
        connection.request("GET",self.path,headers={"Authorization":self.headers.get("Authorization",""),"Accept":"application/json"})
        response=connection.getresponse(); body=response.read(1024*1024+1)
        if len(body)>1024*1024: self.deny(502); connection.close(); return
        self.send_response(response.status); self.send_header("Content-Type",response.getheader("Content-Type","application/json"))
        self.send_header("Content-Length",str(len(body))); self.end_headers(); self.wfile.write(body); connection.close()
    def do_POST(self): self.deny(405)
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
