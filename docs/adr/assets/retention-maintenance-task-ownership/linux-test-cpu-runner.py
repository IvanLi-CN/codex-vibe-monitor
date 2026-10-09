#!/usr/bin/env python3
"""Pin each real test process to a distinct leased guest CPU without changing argv."""
import fcntl
import os
from pathlib import Path
import sys
import time

if len(sys.argv) < 2:
    raise SystemExit("Expected the real test executable and its arguments")

cpus = sorted(os.sched_getaffinity(0))
if "--list" in sys.argv[2:]:
    # Nextest lists regular and ignored cases concurrently; neither runs a test.
    os.sched_setaffinity(0, {cpus[0]})
    os.execvp(sys.argv[1], sys.argv[1:])

slots = Path("/workspace/worktrees/retention-ownership/test-cpu-slots")
slots.mkdir(mode=0o700, exist_ok=True)
offset = os.getpid() % len(cpus)
selected = None
# A test's short-lived descendants can retain the inherited slot after it exits.
# Wait for resource admission before starting the original test and its timers.
deadline = time.monotonic() + 30
while selected is None:
    for cpu in cpus[offset:] + cpus[:offset]:
        descriptor = os.open(slots / f"cpu-{cpu}.lock", os.O_RDWR | os.O_CREAT, 0o600)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            os.close(descriptor)
            continue
        selected = (cpu, descriptor)
        break
    if selected is not None:
        break
    if time.monotonic() >= deadline:
        raise SystemExit("No free guest CPU slot after 30 seconds; test execution is unavailable")
    time.sleep(0.01)

cpu, descriptor = selected
os.sched_setaffinity(0, {cpu})
os.set_inheritable(descriptor, True)
os.execvp(sys.argv[1], sys.argv[1:])
