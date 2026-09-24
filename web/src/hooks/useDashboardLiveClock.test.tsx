/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useDashboardLiveClock } from "./useDashboardLiveClock";

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let originalVisibilityState: DocumentVisibilityState;

function ClockProbe({ enabled = true, testId }: { enabled?: boolean; testId: string }) {
  const nowMs = useDashboardLiveClock(enabled);
  return <output data-testid={testId}>{nowMs}</output>;
}

beforeEach(() => {
  originalVisibilityState = document.visibilityState;
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: "visible",
  });
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: originalVisibilityState,
  });
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("useDashboardLiveClock", () => {
  it("shares one interval across active subscribers and stops it at zero subscribers", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-04T10:05:00.000Z"));
    const setIntervalSpy = vi.spyOn(window, "setInterval");
    const clearIntervalSpy = vi.spyOn(window, "clearInterval");

    act(() => {
      root?.render(
        <>
          <ClockProbe testId="first" />
          <ClockProbe testId="second" />
        </>,
      );
    });

    expect(setIntervalSpy).toHaveBeenCalledTimes(1);
    expect(host?.querySelector('[data-testid="first"]')?.textContent).toBe(
      String(Date.parse("2026-04-04T10:05:00.000Z")),
    );

    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(host?.querySelector('[data-testid="first"]')?.textContent).toBe(
      String(Date.parse("2026-04-04T10:05:01.000Z")),
    );
    expect(host?.querySelector('[data-testid="second"]')?.textContent).toBe(
      String(Date.parse("2026-04-04T10:05:01.000Z")),
    );

    act(() => {
      root?.render(<ClockProbe enabled={false} testId="disabled" />);
    });
    expect(clearIntervalSpy).toHaveBeenCalledTimes(1);
  });

  it("pauses while hidden and resynchronizes when the document becomes visible", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-04T10:05:00.000Z"));

    act(() => {
      root?.render(<ClockProbe testId="clock" />);
    });
    expect(host?.querySelector('[data-testid="clock"]')?.textContent).toBe(
      String(Date.parse("2026-04-04T10:05:00.000Z")),
    );

    Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
    act(() => {
      document.dispatchEvent(new Event("visibilitychange"));
      vi.advanceTimersByTime(2000);
    });
    expect(host?.querySelector('[data-testid="clock"]')?.textContent).toBe(
      String(Date.parse("2026-04-04T10:05:00.000Z")),
    );

    vi.setSystemTime(new Date("2026-04-04T10:05:05.000Z"));
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
    act(() => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    expect(host?.querySelector('[data-testid="clock"]')?.textContent).toBe(
      String(Date.parse("2026-04-04T10:05:05.000Z")),
    );
  });
});
