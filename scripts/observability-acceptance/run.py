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
from environment import AdmissionBudget, actions_context, comparison_report, observe_resources, quiet_admission, verify_measurement_evidence

def execute(arguments, **kwargs):
    return subprocess.check_output(arguments, text=True, timeout=kwargs.pop("timeout",60), **kwargs).strip()
def digest(paths):
    result=hashlib.sha256()
    for path in paths: result.update(path.read_bytes())
    return result.hexdigest()

class Run:
    def __init__(self,args):
        self.args=args; self.source=Path(args.source).resolve();self.root=Path(args.run).resolve()
        self.environment=getattr(args,"environment","shared-testbox")
        self.suite=getattr(args,"suite","runtime")
        self.context=actions_context(self.source,self.root,args.candidate) if self.environment=="github-actions" else None
        if self.suite=="full" and self.context is None:
            raise ValueError("full performance acceptance must run in GitHub Actions")
        if self.suite=="full" and not re.fullmatch(r"sha256:[a-f0-9]{64}",args.image or ""):
            raise ValueError("performance acceptance requires an immutable prebuilt image ID")
        if (self.context is None and not self.root.is_relative_to(Path("/srv/codex/agents")/args.agent)) or self.root==self.source or self.source.is_relative_to(self.root) or self.root.is_relative_to(self.source):
            raise ValueError("acceptance run must be inside the exact Agent Directory")
        if not re.fullmatch(r"[a-z0-9][a-z0-9_-]*",args.agent) or not re.fullmatch(r"[a-f0-9]{40}",args.candidate):
            raise ValueError("invalid agent or candidate identity")
        self.project="testbox-"+args.agent+"-"+hashlib.sha256(str(self.root).encode()).hexdigest()[:16]
        self.compose_file=self.root/"compose.json";self.results={}
        self.image=args.image or self.project+":candidate"
        self.root.mkdir(parents=True,exist_ok=True)
        (self.root/"run-config.json").write_text(json.dumps({"candidate":args.candidate,"requestRate":args.rate,"windowSeconds":args.seconds if self.suite=="full" else None,"warmupSeconds":60 if self.suite=="full" else None,"environment":self.environment,"suite":self.suite,"appCpuQuota":2,"appMemoryLimit":"1g"},indent=2)+"\n")
        if self.context: (self.root/"runner-context.json").write_text(json.dumps(self.context,indent=2)+"\n")
        self.private=self.root/"private";self.private.mkdir(mode=0o700)
        for name in ["metrics-token","read-token","grafana-admin-password"]:
            path=self.private/name;path.write_text(secrets.token_hex(32));path.chmod(0o640)
        self.data=self.root/"data";self.data.mkdir()
        self.data.chmod(0o770)
    def compose(self,*args,**kwargs):
        return execute(["docker","compose","-p",self.project,"-f",str(self.compose_file),*args],**kwargs)
    def client(self,*args):
        with (self.root/"client.log").open("a") as log:
            result=self.compose("exec","-T","client","python","/work/client.py",*args,timeout=600,stderr=log)
        return json.loads(result)
    def checkpoint(self,name,operation):
        try:
            self.results[name]={"status":"passed","result":operation()}
        except Exception as error:
            self.results[name]={"status":"unavailable" if isinstance(error,(OSError,subprocess.TimeoutExpired)) else "failed","error":str(error)}
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
        execute(["openssl","req","-x509","-newkey","rsa:2048","-nodes","-days","1","-subj","/CN=entry","-addext","subjectAltName=DNS:entry","-keyout",str(self.private/"tls.key"),"-out",str(self.private/"tls.crt")],stderr=subprocess.DEVNULL)
        env={**os.environ,"METRICS_TOKEN_FILE":str(self.private/"metrics-token"),"GRAFANA_ADMIN_PASSWORD_FILE":str(self.private/"grafana-admin-password"),"GRAFANA_PUBLIC_URL":"https://entry:8443","OBSERVABILITY_NETWORK":self.project+"-monitoring","OBSERVABILITY_SECRET_GID":str(os.getgid())}
        compose=json.loads(execute(["docker","compose","-p",self.project,"-f",str(self.source/"ops/observability/compose.yml"),"config","--format","json"],env=env))
        compose.pop("name",None);compose["networks"]={"monitoring":{}}
        for volume in compose.get("volumes",{}).values(): volume.pop("name",None)
        for service in compose["services"].values():
            service["cap_drop"]=["ALL"];service.pop("ports",None)
        fixture=self.source/"scripts/observability-acceptance"
        common={"image":"python:3.12-alpine","user":f"{os.getuid()}:{os.getgid()}","cap_drop":["ALL"],"networks":["monitoring"],"volumes":[str(fixture)+":/work:ro",str(self.private)+":/private"]}
        compose["services"].update({
            "app":{"image":self.image,"user":f"0:{os.getgid()}","cap_drop":["ALL"],"cpus":2,"mem_limit":"1g","networks":{"monitoring":{"aliases":["codex-vibe-monitor"]}},"volumes":[str(self.data)+":/srv/app/data",str(self.private/"metrics-token")+":/run/secrets/metrics-token:ro",str(self.private/"read-token")+":/run/secrets/read-token:ro"],"environment":{"DATABASE_PATH":"/srv/app/data/codex_vibe_monitor.db","HTTP_BIND":"0.0.0.0:8080","METRICS_BIND":"0.0.0.0:9091","METRICS_TOKEN_FILE":"/run/secrets/metrics-token","OBSERVABILITY_READ_TOKEN_FILE":"/run/secrets/read-token","GRAFANA_PUBLIC_URL":"https://entry:8443","OBSERVABILITY_ENABLED":"true","UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET":"synthetic-testbox-encryption-secret","RUST_LOG":"warn"}},
            "mock-upstream":{**common,"command":["python","/work/fixture.py","upstream"]},
            "entry":{**common,"command":["python","/work/fixture.py","https"]},
            "client":{**common,"command":["sleep","infinity"]},
        })
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
        self.compose("pull","prometheus","grafana","mock-upstream","entry","client",timeout=600)
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
        self.compose("stop","prometheus","grafana")
        result=self.client("load","--seconds","10","--rate",str(self.args.rate))
        self.compose("up","-d","prometheus","grafana");return result
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
    def cpu_usec(self):
        values=self.compose("exec","-T","app","cat","/sys/fs/cgroup/cpu.stat")
        return int(dict(line.split() for line in values.splitlines())["usage_usec"])
    def overhead(self):
        # An SSH shell or self-hosted Actions job cannot certify the performance budget.
        if self.environment!="github-actions":
            raise ValueError("performance acceptance must run in GitHub Actions")
        actions_context(self.source,self.root,self.args.candidate)
        quiet_admission(self.root)
        return self.overhead_windows()
    def overhead_windows(self):
        # Stop before snapshotting, then use the same seeded state and offered load every round.
        self.compose("stop","app")
        baseline=self.root/"baseline-data";shutil.copytree(self.data,baseline)
        samples={"false":[],"true":[]}
        admission_budget=AdmissionBudget()
        for index in range(3):
            for enabled in ["false","true"] if index%2==0 else ["true","false"]:
                directory=self.root/f"ab-{index}-{enabled}";shutil.copytree(baseline,directory);directory.chmod(0o770)
                # The cap-free app uses the host's group for its synthetic state.
                for path in directory.rglob("*"):
                    assert not path.is_symlink(),"unexpected symlink in synthetic A/B state"
                    path.chmod(0o770 if path.is_dir() else 0o660)
                app=self.definition["services"]["app"]
                app["environment"]["OBSERVABILITY_ENABLED"]=enabled
                app["volumes"][0]=str(directory)+":/srv/app/data"
                self.compose_file.write_text(json.dumps(self.definition,indent=2))
                self.compose("up","-d","app");self.wait_app()
                # Observe two complete 30s resource-sampler cycles before timing.
                self.client("load","--seconds","60","--rate",str(self.args.rate))
                window={"windowId":f"{index}-{enabled}","pairIndex":index,"enabled":enabled}
                window["admissionWaitSeconds"]=quiet_admission(self.root,timeout=300,window=window,budget=admission_budget)
                with observe_resources(self.root,window):
                    before=self.cpu_usec()
                    result=self.client("load","--seconds",str(self.args.seconds),"--rate",str(self.args.rate))
                    result["cpuSecondsPerRequest"]=(self.cpu_usec()-before)/1e6/result["completed"]
                    result["cpuCores"]=result["cpuSecondsPerRequest"]*self.args.rate
                    result["windowId"]=window["windowId"]
                    samples[enabled].append(result)
                    (self.root/"ab-samples.json").write_text(json.dumps(samples,indent=2))
                assert result["cpuCores"]<1.5,"saturated load cannot establish observability overhead"
                assert result["durationSeconds"]<=self.args.seconds*1.05,"request backlog cannot establish non-saturated overhead"
                self.compose("stop","app")
        report=comparison_report(samples)
        (self.root/"ab-summary.json").write_text(json.dumps(report,indent=2)+"\n")
        assert all(metric[mode]["stable"] for metric in report["metrics"].values() for mode in ["false","true"]),"unstable measurement windows"
        assert all(metric["withinBudget"] for metric in report["metrics"].values()),"5% observability budget exceeded"
        return report
    def finish(self):
        expected={"https-auth-query","monitoring-fault-isolation","original-process-cpu"}
        if self.suite=="full": expected.add("default-observability-ab")
        success=all(row["status"]=="passed" for row in self.results.values()) and set(self.results)==expected
        if success and self.suite=="full":
            try:
                verify_measurement_evidence(self.root)
            except OSError:
                self.results["default-observability-ab"]={"status":"unavailable","error":"measurement environment evidence is incomplete or invalid"}
                (self.root/"scenarios.json").write_text(json.dumps(self.results,indent=2)+"\n")
                success=False
        status="passed" if success else "unavailable" if any(row["status"]=="unavailable" for row in self.results.values()) else "failed"
        card={"empirical_acceptance":"required","empirical_acceptance_rationale":"GitHub-hosted exact-image runtime and default CPU/request plus p95 overhead acceptance." if self.suite=="full" else "Runtime integration only; this card does not certify the performance budget.","empirical_evidence_status":status,"empirical_candidate_sha":self.args.candidate,"acceptance_contract_digest":digest([self.source/"docs/specs/performance-telemetry/SPEC.md",self.source/"docs/specs/performance-telemetry/METRICS.md",self.source/"docs/design/performance-observability.md",self.source/"docs/design/performance-observability-metrics.md",self.source/"docs/adr/0025-external-performance-observability.md"]),"scenario_set_digest":digest(sorted((self.source/"scripts/observability-acceptance").glob("*.py"))),"evidence_locator":self.context["evidenceLocator"] if self.context else str(self.root)}
        (self.root/("empirical-card.json" if self.suite=="full" else "runtime-card.json")).write_text(json.dumps(card,indent=2)+"\n")
        return success

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ["source","run","agent","candidate","samply"]: parser.add_argument("--"+name,required=True)
    parser.add_argument("--image");parser.add_argument("--seconds",type=int,default=300);parser.add_argument("--rate",type=int,default=5)
    parser.add_argument("--environment",choices=["shared-testbox","github-actions"],default="shared-testbox")
    parser.add_argument("--suite",choices=["runtime","full"],default="runtime")
    args=parser.parse_args()
    if args.seconds<60 or not 1<=args.rate<=100: parser.error("use at least 60-second windows and a fixed 1..100 request/s rate")
    if args.suite=="full" and (args.environment!="github-actions" or args.seconds<300): parser.error("full acceptance requires GitHub Actions and at least 300-second windows")
    run=Run(args)
    try:
        run.build();run.configure();run.start()
        run.checkpoint("https-auth-query",lambda:run.client("functional"))
        run.checkpoint("monitoring-fault-isolation",run.isolation)
        run.checkpoint("original-process-cpu",run.cpu)
        if args.suite=="full": run.checkpoint("default-observability-ab",run.overhead)
        return 0 if run.finish() else 1
    except Exception as error:
        run.results["setup"]={"status":"unavailable","error":str(error)}
        (run.root/"scenarios.json").write_text(json.dumps(run.results,indent=2)+"\n")
        run.finish()
        print("setup: unavailable",flush=True)
        return 1
    finally:
        if run.compose_file.exists():
            with (run.root/"compose.log").open("w") as log:
                subprocess.run(["docker","compose","-p",run.project,"-f",str(run.compose_file),"logs","--no-color"],stdout=log,stderr=subprocess.STDOUT,timeout=30)
            run.compose("down","--remove-orphans",timeout=60)

if __name__=="__main__": sys.exit(main())
