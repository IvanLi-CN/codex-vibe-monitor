"""One wall-clock budget, including checkout/download time recorded by Actions."""
import math
import os
import signal
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
