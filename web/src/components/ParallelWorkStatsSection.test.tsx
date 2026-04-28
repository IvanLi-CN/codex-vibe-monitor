/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { ParallelWorkStatsResponse } from "../lib/api";
import { ParallelWorkStatsSection } from "./ParallelWorkStatsSection";

vi.mock("./ui/alert", () => ({
  Alert: ({ children }: { children: React.ReactNode }) => (
    <div role="alert">{children}</div>
  ),
}));

vi.mock("../i18n", () => ({
  useTranslation: () => ({
    locale: "en",
    t: (key: string, values?: Record<string, string | number>) => {
      const map: Record<string, string> = {
        "stats.parallelWork.title": "Parallel work",
        "stats.parallelWork.description":
          "Track active prompt-cache conversations.",
        "stats.parallelWork.currentDescription":
          "Uses the page-level range and bucket selection.",
        "stats.parallelWork.loading": "Loading parallel-work buckets…",
        "stats.parallelWork.empty": "No complete buckets yet.",
        "stats.parallelWork.windowToggleAria": "Select parallel-work window",
        "stats.parallelWork.chartAria": `${values?.title ?? ""} time-conversation chart`,
        "stats.parallelWork.samples": `${values?.complete ?? 0} complete buckets · ${values?.active ?? 0} active buckets`,
        "stats.parallelWork.detailsTooltipLabel": `Explain ${values?.title ?? "parallel-work"} details`,
        "stats.parallelWork.rangeSummary": `Range: ${values?.start ?? ""} → ${values?.end ?? ""}`,
        "stats.parallelWork.metrics.min": "Min",
        "stats.parallelWork.metrics.max": "Max",
        "stats.parallelWork.metrics.avg": "Avg",
        "stats.parallelWork.tooltip.parallelCount": "Parallel work",
        "stats.parallelWork.tooltip.requestCount": "Requests",
        "stats.parallelWork.tooltip.conversation":
          "X axis: time · Y axis: conversations",
        "stats.parallelWork.windows.minute7d.title": "Last 7 days · by minute",
        "stats.parallelWork.windows.minute7d.toggleLabel": "7d · minute",
        "stats.parallelWork.windows.minute7d.description": "Minute buckets",
        "stats.parallelWork.windows.hour30d.title": "Last 30 days · by hour",
        "stats.parallelWork.windows.hour30d.toggleLabel": "30d · hour",
        "stats.parallelWork.windows.hour30d.description": "Hour buckets",
        "stats.parallelWork.windows.dayAll.title": "All history · by day",
        "stats.parallelWork.windows.dayAll.toggleLabel": "All · day",
        "stats.parallelWork.windows.dayAll.description": "Day buckets",
        "live.chart.tooltip.instructions":
          "Hover or tap for details. Focus the chart and use arrow keys to switch points.",
      };
      return map[key] ?? key;
    },
  }),
}));

vi.mock("../theme", () => ({
  useTheme: () => ({ themeMode: "dark" }),
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  vi.clearAllMocks();
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function buildTestConversations(
  points: ParallelWorkStatsResponse["minute7d"]["points"],
): ParallelWorkStatsResponse["minute7d"]["conversations"] {
  return [0, 1, 2].map((conversationIndex) => {
    const segments = points
      .filter((_, pointIndex) => (pointIndex + conversationIndex) % 2 === 0)
      .map((point) => ({
        bucketStart: point.bucketStart,
        bucketEnd: point.bucketEnd,
        requestCount: Math.max(1, point.parallelCount),
      }));
    return {
      conversationKey: `test-conversation-${conversationIndex + 1}`,
      label: `C${conversationIndex + 1}`,
      firstSeenAt: segments[0]?.bucketStart ?? "",
      lastSeenAt: segments.at(-1)?.bucketEnd ?? "",
      activeBucketCount: segments.length,
      requestCount: segments.reduce(
        (total, segment) => total + segment.requestCount,
        0,
      ),
      segments,
    };
  });
}

const populatedStats: ParallelWorkStatsResponse = {
  current: {} as ParallelWorkStatsResponse["current"],
  minute7d: {
    rangeStart: "2026-03-01T00:00:00Z",
    rangeEnd: "2026-03-08T00:00:00Z",
    bucketSeconds: 60,
    completeBucketCount: 10080,
    activeBucketCount: 4132,
    minCount: 0,
    maxCount: 18,
    avgCount: 4.67,
    points: [
      {
        bucketStart: "2026-03-07T10:00:00Z",
        bucketEnd: "2026-03-07T10:01:00Z",
        parallelCount: 1,
      },
      {
        bucketStart: "2026-03-07T10:01:00Z",
        bucketEnd: "2026-03-07T10:02:00Z",
        parallelCount: 4,
      },
      {
        bucketStart: "2026-03-07T10:02:00Z",
        bucketEnd: "2026-03-07T10:03:00Z",
        parallelCount: 6,
      },
    ],
    conversations: [],
  },
  hour30d: {
    rangeStart: "2026-02-06T00:00:00Z",
    rangeEnd: "2026-03-08T00:00:00Z",
    bucketSeconds: 3600,
    completeBucketCount: 720,
    activeBucketCount: 321,
    minCount: 0,
    maxCount: 9,
    avgCount: 2.13,
    points: [
      {
        bucketStart: "2026-03-07T00:00:00Z",
        bucketEnd: "2026-03-07T01:00:00Z",
        parallelCount: 0,
      },
      {
        bucketStart: "2026-03-07T01:00:00Z",
        bucketEnd: "2026-03-07T02:00:00Z",
        parallelCount: 2,
      },
      {
        bucketStart: "2026-03-07T02:00:00Z",
        bucketEnd: "2026-03-07T03:00:00Z",
        parallelCount: 3,
      },
    ],
    conversations: [],
  },
  dayAll: {
    rangeStart: "2026-01-01T00:00:00Z",
    rangeEnd: "2026-03-08T00:00:00Z",
    bucketSeconds: 86400,
    completeBucketCount: 67,
    activeBucketCount: 54,
    minCount: 0,
    maxCount: 6,
    avgCount: 2.04,
    points: [
      {
        bucketStart: "2026-03-05T00:00:00Z",
        bucketEnd: "2026-03-06T00:00:00Z",
        parallelCount: 2,
      },
      {
        bucketStart: "2026-03-06T00:00:00Z",
        bucketEnd: "2026-03-07T00:00:00Z",
        parallelCount: 5,
      },
      {
        bucketStart: "2026-03-07T00:00:00Z",
        bucketEnd: "2026-03-08T00:00:00Z",
        parallelCount: 4,
      },
    ],
    conversations: [],
  },
};

populatedStats.minute7d.conversations = buildTestConversations(
  populatedStats.minute7d.points,
);
populatedStats.hour30d.conversations = buildTestConversations(
  populatedStats.hour30d.points,
);
populatedStats.dayAll.conversations = buildTestConversations(
  populatedStats.dayAll.points,
);
populatedStats.current = populatedStats.minute7d;

describe("ParallelWorkStatsSection", () => {
  it("renders the current window card without local period controls", () => {
    render(
      <ParallelWorkStatsSection
        stats={populatedStats}
        isLoading={false}
        error={null}
      />,
    );

    expect(
      host?.querySelector('[data-testid="parallel-work-window-toggle"]'),
    ).toBeNull();
    expect(
      host?.querySelectorAll('[data-testid^="parallel-work-card-"]'),
    ).toHaveLength(1);
    expect(
      host?.querySelector('[data-testid="parallel-work-card-minute7d"]'),
    ).not.toBeNull();
    expect(
      host?.querySelector('[data-testid="parallel-work-card-hour30d"]'),
    ).toBeNull();
    expect(host?.textContent).toContain("Parallel work");
    expect(host?.textContent).toContain("4.67");
    expect(host?.textContent).not.toContain("Last 7 days · by minute");
    expect(host?.textContent).not.toContain("Minute buckets");
    expect(host?.textContent).not.toContain(
      "10080 complete buckets · 4132 active buckets",
    );
    const chart = host?.querySelector(
      '[data-chart-library="recharts"]',
    ) as HTMLElement | null;
    const section = host?.querySelector(
      '[data-testid="parallel-work-section"]',
    ) as HTMLElement | null;
    const heading = host?.querySelector(
      '[data-testid="parallel-work-heading-minute7d"]',
    ) as HTMLElement | null;
    const controls = host?.querySelector(
      '[data-testid="parallel-work-controls-minute7d"]',
    ) as HTMLElement | null;
    const infoTrigger = document.querySelector(
      'button[aria-label="Explain Last 7 days · by minute details"]',
    ) as HTMLButtonElement | null;
    expect(chart).not.toBeNull();
    expect(chart?.getAttribute("data-chart-library")).toBe("recharts");
    expect(chart?.getAttribute("data-chart-kind")).toBe("parallel-work-trend");
    expect(chart?.className).toContain("w-full");
    expect(section).not.toBeNull();
    expect(heading).not.toBeNull();
    expect(controls).toBeNull();
    expect(infoTrigger).not.toBeNull();
    expect(heading?.contains(infoTrigger)).toBe(true);
  });

  it("collapses secondary window copy into a question-mark tooltip", () => {
    render(
      <ParallelWorkStatsSection
        stats={populatedStats}
        isLoading={false}
        error={null}
      />,
    );

    const trigger = document.querySelector(
      'button[aria-label="Explain Last 7 days · by minute details"]',
    ) as HTMLButtonElement | null;
    expect(trigger).not.toBeNull();

    act(() => {
      trigger?.click();
    });

    const tooltip = document.body.querySelector(
      '[role="tooltip"][aria-hidden="false"]',
    ) as HTMLElement;
    expect(tooltip.textContent).toContain("Last 7 days · by minute");
    expect(tooltip.textContent).toContain("Minute buckets");
    expect(tooltip.textContent).toContain(
      "10080 complete buckets · 4132 active buckets",
    );
  });

  it("renders the active window chart through Recharts", () => {
    render(
      <ParallelWorkStatsSection
        stats={populatedStats}
        isLoading={false}
        error={null}
      />,
    );

    const chart = host?.querySelector(
      '[data-chart-library="recharts"]',
    ) as HTMLElement | null;

    expect(chart).not.toBeNull();
    expect(chart?.getAttribute("data-chart-library")).toBe("recharts");
    expect(chart?.getAttribute("data-chart-kind")).toBe("parallel-work-trend");
    expect(chart?.getAttribute("aria-label")).toBe(
      "Last 7 days · by minute time-conversation chart",
    );
  });

  it("uses page range and bucket labels for current chart metadata", () => {
    render(
      <ParallelWorkStatsSection
        stats={populatedStats}
        isLoading={false}
        error={null}
        rangeLabel="Today"
        bucketLabel="Every 5 minutes"
      />,
    );

    const chart = host?.querySelector(
      '[data-chart-library="recharts"]',
    ) as HTMLElement | null;
    const trigger = document.querySelector(
      'button[aria-label="Explain Today · Every 5 minutes details"]',
    ) as HTMLButtonElement | null;

    expect(chart?.getAttribute("aria-label")).toBe(
      "Today · Every 5 minutes time-conversation chart",
    );
    expect(trigger).not.toBeNull();

    act(() => {
      trigger?.click();
    });

    const tooltip = document.body.querySelector(
      '[role="tooltip"][aria-hidden="false"]',
    ) as HTMLElement;
    expect(tooltip.textContent).toContain("Today · Every 5 minutes");
    expect(tooltip.textContent).toContain(
      "Uses the page-level range and bucket selection.",
    );
    expect(tooltip.textContent).not.toContain("Minute buckets");
  });

  it("uses the Gantt-style chart for current windows up to one day", () => {
    render(
      <ParallelWorkStatsSection
        stats={{
          ...populatedStats,
          current: {
            ...populatedStats.minute7d,
            rangeStart: "2026-03-07T00:00:00Z",
            rangeEnd: "2026-03-07T23:59:59Z",
            bucketSeconds: 300,
          },
        }}
        isLoading={false}
        error={null}
      />,
    );

    const chart = host?.querySelector(
      '[data-chart-library="recharts"]',
    ) as HTMLElement | null;
    expect(chart).not.toBeNull();
    expect(chart?.getAttribute("data-chart-kind")).toBe("parallel-work-gantt");
  });

  it("does not render local period controls", () => {
    render(
      <ParallelWorkStatsSection
        stats={populatedStats}
        isLoading={false}
        error={null}
      />,
    );

    expect(
      host?.querySelector('[data-testid="parallel-work-window-toggle"]'),
    ).toBeNull();
    expect(
      host?.querySelector(
        '[data-testid="parallel-work-window-trigger-hour30d"]',
      ),
    ).toBeNull();
  });

  it("renders empty day-all state with null summaries", () => {
    const emptyDayAll: ParallelWorkStatsResponse = {
      ...populatedStats,
      current: {
        rangeStart: "2026-03-08T00:00:00Z",
        rangeEnd: "2026-03-08T00:00:00Z",
        bucketSeconds: 86400,
        completeBucketCount: 0,
        activeBucketCount: 0,
        minCount: null,
        maxCount: null,
        avgCount: null,
        points: [],
        conversations: [],
      },
      dayAll: {
        rangeStart: "2026-03-08T00:00:00Z",
        rangeEnd: "2026-03-08T00:00:00Z",
        bucketSeconds: 86400,
        completeBucketCount: 0,
        activeBucketCount: 0,
        minCount: null,
        maxCount: null,
        avgCount: null,
        points: [],
        conversations: [],
      },
    };

    render(
      <ParallelWorkStatsSection
        stats={emptyDayAll}
        isLoading={false}
        error={null}
        defaultWindowKey="dayAll"
      />,
    );

    const dayAllCard = host?.querySelector(
      '[data-testid="parallel-work-card-dayAll"]',
    );
    expect(dayAllCard?.textContent).toContain("No complete buckets yet.");
    expect(dayAllCard?.textContent).toContain("—");
  });

  it("renders a section-level error alert", () => {
    render(
      <ParallelWorkStatsSection stats={null} isLoading={false} error="boom" />,
    );
    expect(host?.querySelector('[role="alert"]')?.textContent).toContain(
      "boom",
    );
  });
});
