#!/usr/bin/env python3
"""Regression checks for the embedded shared-testbox API smoke client."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SMOKE_SCRIPT = ROOT / "scripts" / "shared-testbox-api-read-smoke"
START = 'cat > "$SMOKE_SCRIPT" <<\'PY\'\n'
END = "\nPY\nchmod +x \"$SMOKE_SCRIPT\""


class Clock:
    def __init__(self) -> None:
        self.now = 0.0
        self.sleeps: list[float] = []

    def monotonic(self) -> float:
        return self.now

    def sleep(self, seconds: float) -> None:
        self.sleeps.append(seconds)
        self.now += seconds


def load_smoke_module() -> dict[str, object]:
    source = SMOKE_SCRIPT.read_text()
    embedded = source.split(START, 1)[1].split(END, 1)[0]
    namespace: dict[str, object] = {"__name__": "shared_testbox_api_read_smoke_test"}
    exec(compile(embedded, str(SMOKE_SCRIPT), "exec"), namespace)
    return namespace


def assert_fixed_deadline() -> None:
    module = load_smoke_module()
    clock = Clock()
    timeouts: list[float] = []

    def request_json(*_args: object, timeout: float, **_kwargs: object) -> tuple[int, str]:
        timeouts.append(timeout)
        clock.now += timeout
        return 503, "summary projection has not completed hydration"

    module["request_json"] = request_json
    try:
        module["wait_summary"](monotonic=clock.monotonic, sleep=clock.sleep)
    except SystemExit as error:
        assert "within 30s" in str(error), error
    else:
        raise AssertionError("summary retry loop unexpectedly succeeded")

    assert clock.now == 30.0, clock.now
    assert timeouts == [5.0] * 5, timeouts
    assert clock.sleeps == [1.0] * 5, clock.sleeps


def assert_late_success_is_rejected() -> None:
    module = load_smoke_module()
    clock = Clock()
    attempts = 0

    def request_json(*_args: object, timeout: float, **_kwargs: object) -> tuple[int, dict[str, int]]:
        nonlocal attempts
        attempts += 1
        clock.now += timeout
        if attempts < 5:
            return 503, "summary projection has not completed hydration"
        clock.now += 2
        return 200, {"totalCount": 0, "successCount": 0, "failureCount": 0}

    module["request_json"] = request_json
    try:
        module["wait_summary"](monotonic=clock.monotonic, sleep=clock.sleep)
    except SystemExit as error:
        assert "within 30s" in str(error), error
    else:
        raise AssertionError("late summary response unexpectedly succeeded")


if __name__ == "__main__":
    assert_fixed_deadline()
    assert_late_success_is_rejected()
    print("test-shared-testbox-api-read-smoke: all checks passed")
