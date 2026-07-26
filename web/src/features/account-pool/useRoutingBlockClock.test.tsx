/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useRoutingBlockClock } from "./useRoutingBlockClock";

function Harness({ until, onExpired }: { until: string; onExpired: () => void }) {
  const nowMs = useRoutingBlockClock([until], onExpired);
  return <output data-testid="now">{nowMs}</output>;
}

describe("useRoutingBlockClock", () => {
  let root: Root | null = null;

  afterEach(() => {
    root?.unmount();
    root = null;
    vi.useRealTimers();
  });

  it("ticks once per second and refreshes once after expiry", () => {
    vi.useFakeTimers();
    const start = Date.parse("2026-07-26T12:00:00.000Z");
    vi.setSystemTime(start);
    const onExpired = vi.fn();
    const container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    act(() => {
      root?.render(<Harness until={new Date(start + 2_000).toISOString()} onExpired={onExpired} />);
    });
    expect(container.textContent).toBe(String(start));
    act(() => void vi.advanceTimersByTime(1_000));
    expect(container.textContent).toBe(String(start + 1_000));
    act(() => void vi.advanceTimersByTime(1_000));
    expect(onExpired).toHaveBeenCalledTimes(1);
    act(() => void vi.advanceTimersByTime(3_000));
    expect(onExpired).toHaveBeenCalledTimes(1);
  });
});
