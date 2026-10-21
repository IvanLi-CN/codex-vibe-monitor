#!/usr/bin/env python3
"""Candidate-bound runtime checks and GitHub-hosted performance acceptance."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import sys
import time
from cpu_attribution import CpuAttributionError, PreparedCpuAttribution
from environment import AdmissionBudget, PAIR_ADMISSION_SECONDS, actions_context, measurement_cpu_layout, observe_resources, quiet_admission, select_measurement_cpu, verify_measurement_evidence
from deadline import AcceptanceDeadlineExceeded, ExecutionDeadline, EXECUTION_SECONDS

FIXTURE_INTERNAL_INGEST_TOKEN = "cvm-fixture-internal-ingest"
FIXTURE_METRICS_TOKEN = "cvm-fixture-metrics-token"
FIXTURE_READ_TOKEN = "cvm-fixture-read-token"
_ANSI_ESCAPE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
_SENSITIVE_VALUE = re.compile(
    r"(?i)(authorization|bearer|basic|token|secret|password|api[_-]?key|cookie|payload|body)"
    r"(\s*[:=]\s*|\s+)(?:(?:bearer|basic)\s+)?[^\s,;)]+"
)
_SAFE_FAILURE_LINE = re.compile(r"(?i)(assertionerror|error|exception|traceback|failed)")

def execute(arguments, **kwargs):
    return subprocess.check_output(arguments, text=True, timeout=kwargs.pop("timeout",60), **kwargs).strip()

def sanitize_client_failure(mode, stderr):
    """Return bounded, non-sensitive client failure evidence for CI artifacts."""
    if isinstance(stderr, bytes):
        stderr = stderr.decode("utf-8", errors="replace")
    text = _ANSI_ESCAPE.sub("", stderr or "")
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    selected = [line for line in lines if _SAFE_FAILURE_LINE.search(line)]
    message = selected[-1] if selected else (lines[-1] if lines else "client exited without diagnostic output")
    message = _SENSITIVE_VALUE.sub(lambda match: match.group(1) + "=[redacted]", message)
    message = re.sub(r"https?://[^\s]+", "[url-redacted]", message)
    return {
        "mode": mode if mode in {"functional", "trace_cases", "trace_stats", "load"} else "unknown",
        "exceptionType": next(
            (name for name in ("AssertionError", "TimeoutExpired", "CalledProcessError", "RuntimeError", "ValueError") if name in message),
            "ClientFailure",
        ),
        "message": message[:512],
        "stderrLineCount": len(lines),
    }

def digest(paths):
    result=hashlib.sha256()
    for path in paths: result.update(path.read_bytes())
    return result.hexdigest()

def certificate(source,candidate,context,status,rationale):
    return {"empirical_acceptance":"required","empirical_acceptance_rationale":rationale,"empirical_evidence_status":status,"empirical_candidate_sha":candidate,"acceptance_contract_digest":digest([source/"docs/specs/performance-telemetry/SPEC.md",source/"docs/specs/performance-telemetry/METRICS.md",source/"docs/design/performance-observability.md",source/"docs/design/performance-observability-metrics.md",source/"docs/adr/0025-external-performance-observability.md",source/"docs/adr/0033-external-request-diagnostic-traces.md",source/"docs/adr/0034-shared-tempo-request-tracing.md",source/"docs/design/request-lifecycle-observability.md"]),"scenario_set_digest":digest(sorted((source/"scripts/observability-acceptance").glob("*.py"))+[source/"ops/observability/tempo_access.py",source/"ops/observability/tempo.yml",source/"ops/observability/compose.yml",source/"ops/observability/grafana/provisioning/datasources/tempo.yml"]),"evidence_locator":context["evidenceLocator"] if context else "runtime-only"}

def measurement_services(enabled):
    if enabled not in {"off", "metrics", "full"}:
        raise ValueError("unknown measurement mode")
    return {
        "prometheus": enabled in {"metrics", "full"},
        "grafana": False,
        "tempo": enabled == "full",
        "entry": enabled == "full",
    }

def create_entry_certificate(private):
    # OpenSSL's default self-signed certificate is a CA. Rustls correctly
    # rejects a CA used as the HTTPS server's end-entity certificate.
    execute(["openssl","req","-x509","-newkey","rsa:2048","-nodes","-days","1","-subj","/CN=entry","-addext","subjectAltName=DNS:entry","-addext","basicConstraints=critical,CA:FALSE","-addext","extendedKeyUsage=serverAuth","-keyout",str(private/"tls.key"),"-out",str(private/"tls.crt")],stderr=subprocess.DEVNULL)

class Run:
    def __init__(self,args):
        self.args=args; self.source=Path(args.source).resolve(); self.candidate_source=Path(getattr(args,"candidate_source",args.source)).resolve(); self.root=Path(args.run).resolve()
        self.environment=getattr(args,"environment","shared-testbox")
        self.suite=getattr(args,"suite","runtime")
        if self.suite not in {"runtime","pair-0","pair-1","pair-2"}:
            raise ValueError("full acceptance is issued only by the all-parts verifier")
        self.pair_index=int(self.suite[-1]) if self.suite.startswith("pair-") else None
        self.context=actions_context(self.candidate_source,self.root,args.candidate) if self.environment=="github-actions" else None
        if self.pair_index is not None and self.context is None:
            raise ValueError("full performance acceptance must run in GitHub Actions")
        if self.pair_index is not None and not re.fullmatch(r"sha256:[a-f0-9]{64}",args.image or ""):
            raise ValueError("performance acceptance requires an immutable prebuilt image ID")
        if (self.context is None and not self.root.is_relative_to(Path("/srv/codex/agents")/args.agent)) or self.root==self.source or self.source.is_relative_to(self.root) or self.root.is_relative_to(self.source):
            raise ValueError("acceptance run must be inside the exact Agent Directory")
        if not re.fullmatch(r"[a-z0-9][a-z0-9_-]*",args.agent) or not re.fullmatch(r"[a-f0-9]{40}",args.candidate):
            raise ValueError("invalid agent or candidate identity")
        if self.pair_index is not None:
            selected_cpu=select_measurement_cpu(self.context["cpuAffinity"])
            self.context["measurementCpu"]=selected_cpu
            self.cpu_layout=measurement_cpu_layout(self.context["cpuAffinity"], selected=selected_cpu)
        else:
            self.cpu_layout=None
        self.project="testbox-"+args.agent+"-"+hashlib.sha256(str(self.root).encode()).hexdigest()[:16]
        self.compose_file=self.root/"compose.json";self.results={}
        self.client_failures=[]
        self.image=args.image or self.project+":candidate"
        self.root.mkdir(parents=True,exist_ok=True)
        self.execution_started_utc=float(os.environ.get("OBSERVABILITY_TEST_STARTED_UTC",time.time()))
        self.execution_deadline_utc=self.execution_started_utc+EXECUTION_SECONDS
        (self.root/"run-config.json").write_text(json.dumps({"candidate":args.candidate,"requestRate":args.rate,"windowSeconds":args.seconds if self.pair_index is not None else None,"warmupSeconds":60 if self.pair_index is not None else None,"pairIndex":self.pair_index,"environment":self.environment,"suite":self.suite,"appCpuQuota":1 if self.cpu_layout else 2,"appCpuSet":self.cpu_layout["app"] if self.cpu_layout else None,"auxiliaryCpuSet":self.cpu_layout["auxiliary"] if self.cpu_layout else None,"appMemoryLimit":"1g","executionStartedUTCSeconds":float(os.environ.get("OBSERVABILITY_TEST_STARTED_UTC",time.time()))},indent=2)+"\n")
        if self.context: (self.root/"runner-context.json").write_text(json.dumps(self.context,indent=2)+"\n")
        self.private=self.root/"private";self.private.mkdir(mode=0o700)
        for name in ["grafana-admin-password","tempo-ingest-token","tempo-query-token"]:
            path=self.private/name;path.write_text(secrets.token_hex(32));path.chmod(0o640)
        for name, value in [("metrics-token", FIXTURE_METRICS_TOKEN), ("read-token", FIXTURE_READ_TOKEN)]:
            path=self.private/name;path.write_text(value + "\n");path.chmod(0o640)
        runtime_token=self.private/"tempo-runtime-token"
        runtime_token.write_text(FIXTURE_INTERNAL_INGEST_TOKEN + "\n")
        runtime_token.chmod(0o640)
        self.data=self.root/"data";self.data.mkdir()
        self.data.chmod(0o770)
    def compose(self,*args,**kwargs):
        return execute(["docker","compose","-p",self.project,"-f",str(self.compose_file),*args],**kwargs)
    def client(self,*args):
        # SIGALRM interrupts only the main thread. A CPU sampler's concurrent
        # load must also stop by the same deadline before executor shutdown.
        remaining=self.execution_deadline_utc-time.time()-5
        if remaining<=0:
            raise AcceptanceDeadlineExceeded("acceptance client deadline exhausted")
        try:
            result=self.compose("exec","-T","client","python","/work/client.py",*args,timeout=min(600,remaining),stderr=subprocess.PIPE)
        except subprocess.CalledProcessError as error:
            self.client_failures.append(sanitize_client_failure(args[-1] if args else "unknown", error.stderr))
            self._write_client_failures()
            raise
        except subprocess.TimeoutExpired as error:
            self.client_failures.append(sanitize_client_failure(args[-1] if args else "unknown", error.stderr))
            self._write_client_failures()
            raise
        return json.loads(result)
    def _write_client_failures(self):
        (self.root/"client-failures.json").write_text(json.dumps({"failures":self.client_failures},indent=2)+"\n")
    def checkpoint(self,name,operation):
        try:
            self.results[name]={"status":"passed","result":operation()}
        except AcceptanceDeadlineExceeded:
            raise
        except Exception as error:
            self.results[name]={"status":"unavailable" if isinstance(error,(OSError,subprocess.TimeoutExpired)) else "failed","error":str(error)}
            print(name+" error: "+str(error), flush=True)
            self._write_client_failures()
        (self.root/"scenarios.json").write_text(json.dumps(self.results,indent=2)+"\n")
        # A/B recreates the application; preserve each scenario's diagnostics first.
        with (self.root/(name+"-compose.log")).open("w") as log:
            subprocess.run(["docker","compose","-p",self.project,"-f",str(self.compose_file),"logs","--no-color"],stdout=log,stderr=subprocess.STDOUT,timeout=30)
        print(name+": "+self.results[name]["status"],flush=True)
    def build(self):
        if not self.args.image:
            with (self.root/"build.log").open("w") as log:
                subprocess.run(["docker","build","--target","runtime","--build-arg","APP_GIT_REVISION="+self.args.candidate,"--build-arg","APP_EFFECTIVE_VERSION=observability-acceptance","-t",self.image,str(self.source)],stdout=log,stderr=subprocess.STDOUT,check=True,timeout=3600)
        identity=json.loads(execute(["docker","image","inspect",self.image]))[0]
        assert identity["Config"]["Labels"]["org.opencontainers.image.revision"]==self.args.candidate,"image does not match Candidate SHA"
        (self.root/"image-identity.json").write_text(json.dumps(identity,indent=2)+"\n")
    def configure(self):
        create_entry_certificate(self.private)
        env={**os.environ,"METRICS_TOKEN_FILE":str(self.private/"metrics-token"),"GRAFANA_ADMIN_PASSWORD_FILE":str(self.private/"grafana-admin-password"),"GRAFANA_PUBLIC_URL":"https://entry:8443","OBSERVABILITY_NETWORK":self.project+"-monitoring","OBSERVABILITY_TEMPO_NETWORK":self.project+"-tempo-backend","OBSERVABILITY_SECRET_GID":str(os.getgid()),"CVM_TEMPO_QUERY_URL":"https://entry:8443/tempo","CVM_TEMPO_QUERY_TOKEN":(self.private/"tempo-query-token").read_text().strip(),"CVM_TEMPO_CA_PEM":(self.private/"tls.crt").read_text()}
        compose=json.loads(execute(["docker","compose","-p",self.project,"-f",str(self.source/"ops/observability/compose.yml"),"--profile","traces-isolation","config","--format","json"],env=env))
        compose.pop("name",None)
        # Keep both networks internal. The candidate receives only a fixed,
        # non-secret fixture tokens for the app's own auth and isolated ingest
        # route; runner-generated credentials remain mounted only in helpers.
        compose["networks"]={"monitoring":{"internal":True},"tempo_backend":{"internal":True}}
        for volume in compose.get("volumes",{}).values(): volume.pop("name",None)
        for service in compose["services"].values():
            service["cap_drop"]=["ALL"];service.pop("ports",None);service.pop("profiles",None)
            # Keep helpers runner-managed; pinning them can create host PSI pressure.
        fixture=self.source/"scripts/observability-acceptance"
        common={"image":"python:3.12-alpine","user":f"{os.getuid()}:{os.getgid()}","cap_drop":["ALL"],"networks":["monitoring"],"volumes":[str(fixture)+":/work:ro",str(self.source/"ops/observability")+":/observability:ro",str(self.private)+":/private"]}
        compose["services"].update({
            "app":{"image":self.image,"user":f"0:{os.getgid()}","cap_drop":["ALL"],"cpus":2,"mem_limit":"1g","networks":{"monitoring":{"aliases":["codex-vibe-monitor"]}},"volumes":[str(self.data)+":/srv/app/data",str(self.private/"metrics-token")+":/run/secrets/metrics-token:ro",str(self.private/"read-token")+":/run/secrets/read-token:ro",str(self.private/"tempo-runtime-token")+":/run/secrets/tempo-runtime-token:ro",str(self.private/"tls.crt")+":/run/secrets/tempo-ca.crt:ro"],"environment":{"DATABASE_PATH":"/srv/app/data/codex_vibe_monitor.db","HTTP_BIND":"0.0.0.0:8080","METRICS_BIND":"0.0.0.0:9091","METRICS_TOKEN_FILE":"/run/secrets/metrics-token","OBSERVABILITY_READ_TOKEN_FILE":"/run/secrets/read-token","GRAFANA_PUBLIC_URL":"https://entry:8443","OBSERVABILITY_ENABLED":"true","OBSERVABILITY_TRACES_ENABLED":"true","OBSERVABILITY_OTLP_TRACES_ENDPOINT":"https://entry:8443/v1/traces","OBSERVABILITY_OTLP_TOKEN_FILE":"/run/secrets/tempo-runtime-token","SSL_CERT_FILE":"/run/secrets/tempo-ca.crt","OBSERVABILITY_ENVIRONMENT":"production","OBSERVABILITY_INSTANCE":"primary","UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET":"synthetic-testbox-encryption-secret","RUST_LOG":"warn"}},
            "mock-upstream":{**common,"command":["python","/work/fixture.py","upstream"]},
            "entry":{**common,"command":["python","/work/fixture.py","https"],"networks":["monitoring","tempo_backend"]},
            "client":{**common,"command":["sleep","infinity"]},
        })
        if self.cpu_layout:
            compose["services"]["app"]["cpus"]=1
            compose["services"]["app"]["cpuset"]=self.cpu_layout["app"]
        # Exercise the documented .env.local surface before hotpath/Tokio startup.
        app=compose["services"]["app"]
        local_names=["METRICS_BIND","METRICS_TOKEN_FILE","OBSERVABILITY_READ_TOKEN_FILE","GRAFANA_PUBLIC_URL","OBSERVABILITY_ENABLED"]
        local_env=self.private/"app.env.local"
        local_env.write_text("".join(name+"="+app["environment"].pop(name)+"\n" for name in local_names));local_env.chmod(0o640)
        app["volumes"].append(str(local_env)+":/srv/app/.env.local:ro")
        self.definition=compose;self.compose_file.write_text(json.dumps(compose,indent=2))
    def wait_app(self):
        deadline=time.monotonic()+120
        while time.monotonic()<deadline:
            try:
                if self.compose("exec","-T","app","curl","-fsS","http://127.0.0.1:8080/health",stderr=subprocess.DEVNULL)=="ok": return
            except subprocess.SubprocessError: pass
            time.sleep(1)
        raise TimeoutError("application readiness timeout")
    def start(self):
        # Pull before any measured window, including the client image used by exec.
        if self.pair_index is not None:
            self.compose("pull","prometheus","tempo","mock-upstream","entry","client",timeout=180)
            # Seed with observation disabled; runtime scenarios have their own runner.
            self.definition["services"]["app"]["environment"].update(OBSERVABILITY_ENABLED="false",OBSERVABILITY_TRACES_ENABLED="false")
            self.compose_file.write_text(json.dumps(self.definition,indent=2))
            self.compose("up","-d","app","mock-upstream","client",timeout=90);self.wait_app()
            self.client("seed")
            return
        self.compose("pull","prometheus","grafana","tempo","mock-upstream","entry","client",timeout=600)
        self.compose("up","-d",timeout=180);self.wait_app()
        self.client("seed")
        deadline=time.monotonic()+120
        while True:
            try: self.client("ready");break
            except subprocess.SubprocessError:
                if time.monotonic()>deadline: raise
                time.sleep(2)
        self.client("viewer")
        self.client("browser_seed")
        self.client("load","--seconds","40","--rate",str(self.args.rate))
    def isolation(self):
        self.compose("stop","prometheus","grafana","tempo")
        result=self.client("load","--seconds","10","--rate",str(self.args.rate))
        self.compose("up","-d","prometheus","grafana","tempo");return result
    def cpu(self):
        # Export symbols from this exact running image before attaching its original process.
        symbols=self.root/"cpu/symbols";symbols.mkdir(parents=True);symbols.chmod(0o770)
        temporary=self.root/"cpu-tmp";temporary.mkdir()
        execute(["bash",str(self.source/".github/scripts/export-image-symbols.sh"),self.image,self.args.candidate,str(symbols)],timeout=120,env={**os.environ,"TMPDIR":str(temporary)})
        profiles=self.root/"cpu/profiles";profiles.mkdir();profiles.chmod(0o770)
        container=self.compose("ps","-q","app")
        build=self.root/"cpu-build";build.mkdir()
        sampler_build=build/"sampler";sampler_build.mkdir()
        samply=Path(self.args.samply).resolve(strict=True)
        shutil.copyfile(samply,sampler_build/"samply")
        shutil.copyfile(self.source/"ops/observability/cpu/Dockerfile",sampler_build/"Dockerfile")
        sampler_image=self.project+":sampler"
        with (self.root/"sampler-build.log").open("w") as log:
            subprocess.run(["docker","build","--label","codex.testbox.agent="+self.args.agent,"-t",sampler_image,str(sampler_build)],stdout=log,stderr=subprocess.STDOUT,check=True,timeout=600)
        sampler_id=execute(["docker","image","inspect","--format","{{.Id}}",sampler_image])
        (build/"binding.json").write_text(json.dumps({"container":container,"profileRoot":str(profiles),"symbolRoot":str(symbols),"profilerImage":sampler_id}))
        shutil.copyfile(self.source/"scripts/cvm-hotpath-cpu",build/"capture.py")
        # The driver has no perf capability; only the fixed sampler can attach.
        (build/"Dockerfile").write_text("FROM ubuntu:24.04\nRUN apt-get update && apt-get install -y --no-install-recommends python3 binutils ca-certificates && rm -rf /var/lib/apt/lists/*\nCOPY binding.json /etc/cvm-observability/cpu.json\nCOPY capture.py /capture.py\nUSER 0:"+str(os.getgid())+"\nENTRYPOINT [\"python3\",\"/capture.py\"]\n")
        profiler_image=self.project+":profiler"
        with (self.root/"profiler-build.log").open("w") as log:
            subprocess.run(["docker","build","-t",profiler_image,str(build)],stdout=log,stderr=subprocess.STDOUT,check=True,timeout=600)
        profiler=self.project+"-profiler"
        command=["docker","run","--name",profiler,"--label","codex.testbox.agent="+self.args.agent,"--cap-drop=ALL","--pid=host","-v",str(self.root)+":"+str(self.root),"-v","/usr/bin/docker:/usr/local/bin/docker:ro","-v","/var/run/docker.sock:/var/run/docker.sock",profiler_image,"capture","30"]
        with ThreadPoolExecutor(max_workers=1) as executor:
            load=executor.submit(self.client,"load","--seconds","40","--rate",str(self.args.rate))
            try:
                with (self.root/"cpu-capture.log").open("w") as log:
                    subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,check=True,timeout=180)
                copied=self.root/"cpu-verified";copied.mkdir()
                execute(["docker","cp",profiler+":"+str(profiles)+"/.",str(copied)],timeout=30)
            finally: execute(["docker","rm","-f",profiler],timeout=20)
            load.result()
        manifests=list(copied.glob("*.manifest.json"));assert len(manifests)==1
        manifest=json.loads(manifests[0].read_text())
        assert manifest["containerId"]==json.loads(execute(["docker","inspect",container]))[0]["Id"]
        assert manifest["durationSeconds"]==30 and manifest["revision"]==self.args.candidate
        sidecar=(copied/manifest["symbols"]).read_text()
        assert "codex_vibe_monitor" in sidecar and ("proxy" in sidecar or "sqlite_batch_writer" in sidecar),"known application hot path was not resolved"
        return {"buildId":manifest["buildId"],"originalContainerId":manifest["containerId"],"durationSeconds":30,"symbols":"resolved"}
    def prepare_cpu_attribution(self, enabled):
        if self.context is None or self.pair_index is None:
            raise ValueError("CPU attribution requires a paired GitHub Actions window")
        services = ["app", "mock-upstream", "client"]
        services.extend(service for service, running in measurement_services(enabled).items() if running)
        containers = []
        for service in services:
            container = self.compose("ps", "-q", service)
            if not re.fullmatch(r"[a-f0-9]{12,64}", container):
                raise OSError("CPU attribution preparation missing running container")
            metadata = json.loads(execute(["docker", "inspect", container]))[0]
            if not metadata.get("State", {}).get("Running") or not isinstance(metadata.get("State", {}).get("Pid"), int):
                raise OSError("CPU attribution preparation found a stopped container")
            containers.append({"service": service, "containerId": metadata["Id"], "pid": metadata["State"]["Pid"]})
        try:
            return PreparedCpuAttribution(
                self.context["cpuAffinity"],
                self.context["measurementCpu"],
                containers,
            )
        except CpuAttributionError as error:
            raise OSError("CPU attribution preparation unavailable: " + error.code) from error
    def overhead(self):
        # An SSH shell or self-hosted Actions job cannot certify the performance budget.
        if self.environment!="github-actions":
            raise ValueError("performance acceptance must run in GitHub Actions")
        actions_context(self.candidate_source,self.root,self.args.candidate)
        # Each measured window performs strict post-warmup admission. A second
        # initial quiet wait would only charge the same idle boundary twice.
        return self.overhead_windows()
    def overhead_windows(self):
        # Stop before snapshotting, then use the same seeded state and offered load every round.
        self.compose("stop","app")
        baseline=self.root/"baseline-data";shutil.copytree(self.data,baseline)
        samples={"off":[],"metrics":[],"full":[]}
        admission_budget=AdmissionBudget(PAIR_ADMISSION_SECONDS)
        for index in [self.pair_index]:
            # Rotate positions so no mode always receives the cold/last window.
            for enabled in (("off","metrics","full"),("metrics","full","off"),("full","off","metrics"))[index]:
                directory=self.root/f"ab-{index}-{enabled}";shutil.copytree(baseline,directory);directory.chmod(0o770)
                # The cap-free app uses the host's group for its synthetic state.
                for path in directory.rglob("*"):
                    assert not path.is_symlink(),"unexpected symlink in synthetic A/B state"
                    path.chmod(0o770 if path.is_dir() else 0o660)
                app=self.definition["services"]["app"]
                app["environment"]["OBSERVABILITY_ENABLED"]="false" if enabled=="off" else "true"
                app["environment"]["OBSERVABILITY_TRACES_ENABLED"]="true" if enabled=="full" else "false"
                app["volumes"][0]=str(directory)+":/srv/app/data"
                self.compose_file.write_text(json.dumps(self.definition,indent=2))
                for service, running in measurement_services(enabled).items():
                    if running:
                        self.compose("up","-d",service)
                    else:
                        self.compose("stop",service)
                self.compose("up","-d","app");self.wait_app()
                attribution=self.prepare_cpu_attribution(enabled)
                # Observe two complete 30s resource-sampler cycles before timing.
                self.client("load","--seconds","60","--rate",str(self.args.rate))
                window={"windowId":f"{index}-{enabled}","pairIndex":index,"enabled":enabled}
                window["admissionWaitSeconds"]=quiet_admission(self.root,timeout=180,window=window,budget=admission_budget)
                with observe_resources(self.root,window,attribution) as observer:
                    before=observer.cpu_usage_usec()
                    result=self.client("load","--seconds",str(self.args.seconds),"--rate",str(self.args.rate))
                    result["cpuSecondsPerRequest"]=(observer.cpu_usage_usec()-before)/1e6/result["completed"]
                    result["cpuCores"]=result["cpuSecondsPerRequest"]*self.args.rate
                    result["windowId"]=window["windowId"]
                    if enabled=="full":
                        result["traceEvidence"]=self.client("trace_stats")
                        assert result["traceEvidence"]["enabled"] and result["traceEvidence"]["exportedSpans"]>0, "full mode did not export real application traces"
                        assert result["traceEvidence"]["failedSpans"]==0 and result["traceEvidence"]["droppedSpans"]==0, "trace loss cannot establish the budget"
                    samples[enabled].append(result)
                    (self.root/"ab-samples.json").write_text(json.dumps(samples,indent=2))
                assert result["cpuCores"]<1.5,"saturated load cannot establish observability overhead"
                assert result["durationSeconds"]<=self.args.seconds*1.05,"request backlog cannot establish non-saturated overhead"
                self.compose("stop","app")
        verify_measurement_evidence(self.root,pair_index=self.pair_index)
        return {"pairIndex":self.pair_index,"windows":3}
    def finish(self):
        config_path=self.root/"run-config.json"
        if config_path.exists():
            config=json.loads(config_path.read_text());config["executionFinishedUTCSeconds"]=time.time()
            config_path.write_text(json.dumps(config,indent=2)+"\n")
        expected={"https-auth-query","tempo-cases-tenant","monitoring-fault-isolation","original-process-cpu"}
        if getattr(self,"pair_index",None) is not None: expected={"default-observability-ab"}
        success=all(row["status"]=="passed" for row in self.results.values()) and set(self.results)==expected
        status="passed" if success else "unavailable" if any(row["status"]=="unavailable" for row in self.results.values()) else "failed"
        card=certificate(self.source,self.args.candidate,self.context,status,"Runtime/paired partial evidence only; this card does not certify the performance budget.")
        (self.root/"runtime-card.json").write_text(json.dumps(card,indent=2)+"\n")
        return success

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ["source","run","agent","candidate","samply"]: parser.add_argument("--"+name,required=True)
    parser.add_argument("--candidate-source")
    parser.add_argument("--image");parser.add_argument("--seconds",type=int,default=60);parser.add_argument("--rate",type=int,default=5)
    parser.add_argument("--environment",choices=["shared-testbox","github-actions"],default="shared-testbox")
    parser.add_argument("--suite",choices=["runtime","pair-0","pair-1","pair-2"],default="runtime")
    args=parser.parse_args()
    if args.seconds<60 or not 1<=args.rate<=100: parser.error("use at least 60-second windows and a fixed 1..100 request/s rate")
    if args.suite!="runtime" and (args.environment!="github-actions" or args.seconds!=60 or args.rate!=5): parser.error("paired acceptance requires Actions, 60-second windows and 5 request/s")
    run=Run(args)
    try:
        with ExecutionDeadline():
            run.build();run.configure();run.start()
            if run.pair_index is None:
                run.checkpoint("https-auth-query",lambda:run.client("functional"))
                run.checkpoint("tempo-cases-tenant",lambda:run.client("trace_cases"))
                run.checkpoint("monitoring-fault-isolation",run.isolation)
                run.checkpoint("original-process-cpu",run.cpu)
            else:
                run.checkpoint("default-observability-ab",run.overhead)
            return 0 if run.finish() else 1
    except Exception as error:
        scenario="default-observability-ab" if run.pair_index is not None else "setup"
        run.results[scenario]={"status":"unavailable","error":str(error)}
        (run.root/"scenarios.json").write_text(json.dumps(run.results,indent=2)+"\n")
        run.finish()
        print("setup: unavailable",flush=True)
        return 1
    finally:
        if run.compose_file.exists():
            try:
                with (run.root/"compose.log").open("w") as log:
                    subprocess.run(["docker","compose","-p",run.project,"-f",str(run.compose_file),"logs","--no-color"],stdout=log,stderr=subprocess.STDOUT,timeout=10)
            finally:
                run.compose("down","--remove-orphans",timeout=30)

if __name__=="__main__": sys.exit(main())
