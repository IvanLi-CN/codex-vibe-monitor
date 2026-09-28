/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import SystemPerformancePage from "./SystemPerformancePage";

const apiMocks = vi.hoisted(() => ({
  fetchPerformanceHealth: vi.fn(),
  fetchPerformanceMetrics: vi.fn(),
}));

vi.mock("../../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../../lib/api")>("../../lib/api");
  return {
    ...actual,
    fetchPerformanceHealth: apiMocks.fetchPerformanceHealth,
    fetchPerformanceMetrics: apiMocks.fetchPerformanceMetrics,
  };
});

vi.mock("recharts", () => ({
  CartesianGrid: () => null,
  Line: () => null,
  LineChart: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  ResponsiveContainer: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  Tooltip: () => null,
  XAxis: () => null,
  YAxis: () => null,
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

describe("SystemPerformancePage", () => {
  beforeEach(() => {
    apiMocks.fetchPerformanceMetrics.mockResolvedValue({
      from: "2026-06-22T00:00:00Z",
      to: "2026-06-22T01:00:00Z",
      stepSeconds: 60,
      coverage: 0.75,
      epochs: ["epoch-1"],
      series: [
        {
          metricId: "p1.queue_depth",
          section: "storage",
          dimension: "p1",
          kind: "gauge",
          unit: "count",
          points: [
            {
              bucketStart: 1_750_536_000,
              sampleCount: 2,
              expectedCount: 2,
              sum: 20,
              min: 8,
              max: 12,
              last: 12,
            },
          ],
        },
        ...Array.from({ length: 12 }, (_, index) => ({
          metricId: `storage.synthetic_${index}`,
          section: "storage" as const,
          dimension: "p1",
          kind: "gauge" as const,
          unit: "count" as const,
          points: [
            {
              bucketStart: 1_750_536_000 + index,
              sampleCount: 1,
              expectedCount: 1,
              sum: index + 1,
              min: index + 1,
              max: index + 1,
              last: index + 1,
            },
          ],
        })),
      ],
    });
    apiMocks.fetchPerformanceHealth.mockResolvedValue({
      state: "healthy",
      enabled: true,
      path: "performance.sqlite",
      epoch: "epoch-1",
      queueDepth: 0,
      queueCapacity: 2048,
      droppedSamples: 0,
      flushFailureCount: 0,
    });
  });

  afterEach(() => {
    act(() => root?.unmount());
    host?.remove();
    host = null;
    root = null;
    apiMocks.fetchPerformanceHealth.mockReset();
    apiMocks.fetchPerformanceMetrics.mockReset();
  });

  it("shows coverage, collector state, and bounded metric series", async () => {
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    await act(async () => {
      root?.render(
        <I18nProvider initialLocale="zh" persistLocale={false}>
          <SystemPerformancePage />
        </I18nProvider>,
      );
      await Promise.resolve();
    });

    expect(host.textContent).toContain("性能遥测");
    expect(host.textContent).toContain("覆盖率");
    expect(host.textContent).toContain("75%");
    expect(host.textContent).toContain("p1.queue_depth");
    expect(host.querySelectorAll('[data-testid="performance-series"]')).toHaveLength(13);
  });

  it("shows an explicit error when the collector health endpoint fails", async () => {
    apiMocks.fetchPerformanceHealth.mockRejectedValueOnce(new Error("health unavailable"));
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    await act(async () => {
      root?.render(
        <I18nProvider initialLocale="zh" persistLocale={false}>
          <SystemPerformancePage />
        </I18nProvider>,
      );
      await Promise.resolve();
    });

    expect(host.querySelector('[data-testid="system-performance-health-error"]')).not.toBeNull();
    expect(host.textContent).toContain("health unavailable");
  });
});
