"""One wall-clock budget, including checkout/download time recorded by Actions."""
import math
import os
import signal
import subprocess
import sys
import time

TEST_JOB_SECONDS = 900
# Reserve one minute for cleanup and whitelisted evidence; Actions also enforces
# the unconditional 900s job limit if a child or Docker cleanup is unresponsive.
EXECUTION_SECONDS = 840


class AcceptanceDeadlineExceeded(OSError):
    pass


class ExecutionDeadline:
    def __init__(self):
        self.started_utc = float(os.environ.get("OBSERVABILITY_TEST_STARTED_UTC", time.time()))
        now = time.time()
        if not math.isfinite(self.started_utc) or self.started_utc > now:
            raise ValueError("invalid acceptance start time")
        self.remaining = min(EXECUTION_SECONDS, self.started_utc + EXECUTION_SECONDS - now)

    def expired(self, *_):
        raise AcceptanceDeadlineExceeded("acceptance execution deadline exhausted")

    def __enter__(self):
        if self.remaining <= 0:
            self.expired()
        self.previous = signal.signal(signal.SIGALRM, self.expired)
        signal.setitimer(signal.ITIMER_REAL, self.remaining)
        return self

    def __exit__(self, *_):
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, self.previous)


def run_preparation(command):
    """Bound an entire preparation process group, including curl retries."""
    remaining = ExecutionDeadline().remaining
    if remaining <= 0:
        raise AcceptanceDeadlineExceeded("acceptance preparation deadline exhausted")
    process = subprocess.Popen(command, start_new_session=True)
    try:
        result = process.wait(timeout=remaining)
    except BaseException:
        # Only this newly created process group is owned by this invocation.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait(timeout=5)
        raise
    if result:
        raise subprocess.CalledProcessError(result, command)
    return 0


if __name__ == "__main__":
    if len(sys.argv) < 3 or sys.argv[1] != "--":
        raise SystemExit("usage: deadline.py -- command [argument ...]")
    raise SystemExit(run_preparation(sys.argv[2:]))
