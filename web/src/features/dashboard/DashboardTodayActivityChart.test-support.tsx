/** @vitest-environment jsdom */
import type { ReactNode } from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, vi } from "vitest";
import { resetDashboardPerformanceDiagnostics } from "../../lib/dashboardPerformanceDiagnostics";

let latestChartData: Array<Record<string, unknown>> = [];
const storage = new Map<string, string>();
const localStorageMock = {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => {
    storage.set(key, value);
  },
  removeItem: (key: string) => {
    storage.delete(key);
  },
  clear: () => {
    storage.clear();
  },
};
let host: HTMLDivElement | null = null;
let root: Root | null = null;
const rechartsMock = {
  ResponsiveContainer: ({ children }: { children: ReactNode }) => (
    <div data-testid="responsive">{children}</div>
  ),
  CartesianGrid: () => <div data-testid="grid" />,
  XAxis: ({ domain }: { domain?: [number, number] }) => (
    <div data-testid="x-axis" data-domain={domain == null ? "" : domain.join(":")} />
  ),
  YAxis: ({
    yAxisId,
    tickFormatter,
  }: {
    yAxisId?: string;
    tickFormatter?: (value: number) => string;
  }) => (
    <div
      data-testid="y-axis"
      data-y-axis-id={yAxisId ?? ""}
      data-negative-tick={tickFormatter?.(-42) ?? ""}
    />
  ),
  Tooltip: ({
    content,
  }: {
    content?: (props: {
      active: boolean;
      label: number;
      payload: Array<{ payload: Record<string, unknown> }>;
    }) => ReactNode;
  }) => {
    const point =
      latestChartData.find(
        (item) => typeof item.inFlightCount === "number" && Number(item.inFlightCount) > 0,
      ) ?? latestChartData.find((item) => typeof item.chartSuccessCount === "number");
    return (
      <div data-testid="tooltip">
        {point
          ? content?.({
              active: true,
              label: Number(point.index ?? 0),
              payload: [{ payload: point }],
            })
          : null}
      </div>
    );
  },
  Legend: () => <div data-testid="legend" />,
  ReferenceLine: () => <div data-testid="reference-line" />,
  Area: ({
    dataKey,
    yAxisId,
    name,
    strokeWidth,
    stackId,
  }: {
    dataKey?: string;
    yAxisId?: string;
    name?: string;
    strokeWidth?: number;
    stackId?: string;
  }) => (
    <div
      data-testid="area-series"
      data-data-key={dataKey ?? ""}
      data-y-axis-id={yAxisId ?? ""}
      data-name={name ?? ""}
      data-stroke-width={String(strokeWidth ?? "")}
      data-stack-id={stackId ?? ""}
    />
  ),
  Line: ({
    data,
    dataKey,
    dot,
    yAxisId,
    name,
    strokeWidth,
    strokeOpacity,
    connectNulls,
  }: {
    data?: Array<Record<string, unknown>>;
    dataKey?: string;
    dot?: false | Record<string, unknown>;
    yAxisId?: string;
    name?: string;
    strokeWidth?: number;
    strokeOpacity?: number;
    connectNulls?: boolean;
  }) => (
    <div
      data-testid="line-series"
      data-data-key={dataKey ?? ""}
      data-y-axis-id={yAxisId ?? ""}
      data-name={name ?? ""}
      data-stroke-width={String(strokeWidth ?? "")}
      data-stroke-opacity={String(strokeOpacity ?? "")}
      data-dot={dot === false ? "false" : dot ? "visible" : ""}
      data-connect-nulls={String(connectNulls ?? "")}
      data-data-length={String(data?.length ?? "")}
    />
  ),
  Bar: ({
    stackId,
    dataKey,
    barSize,
    radius,
    shape,
  }: {
    stackId?: string;
    dataKey?: string;
    barSize?: number;
    radius?: number[];
    shape?: ReactNode;
  }) => (
    <div
      data-testid="bar-series"
      data-stack-id={stackId ?? ""}
      data-data-key={dataKey ?? ""}
      data-bar-size={String(barSize ?? "")}
      data-radius={radius == null ? "" : radius.join(":")}
      data-has-shape={shape == null ? "false" : "true"}
    />
  ),
  AreaChart: ({ children }: { children: ReactNode }) => (
    <div data-testid="area-chart">{children}</div>
  ),
  ComposedChart: ({
    children,
    barGap,
    data,
    stackOffset,
  }: {
    children: ReactNode;
    barGap?: string | number;
    data?: Array<Record<string, unknown>>;
    stackOffset?: string;
  }) => {
    latestChartData = data ?? [];
    return (
      <div
        data-testid="composed-chart"
        data-bar-gap={String(barGap ?? "")}
        data-stack-offset={stackOffset ?? ""}
        data-data-length={String(latestChartData.length)}
      >
        {children}
      </div>
    );
  },
};
vi.mock("recharts", () => rechartsMock);
vi.mock("../../i18n", () => ({
  useTranslation: () => ({
    locale: "zh",
    t: (key: string) => key,
  }),
}));
vi.mock("../../theme", () => ({
  useTheme: () => ({
    themeMode: "light",
  }),
}));
const response = {
  rangeStart: "2026-04-08 00:00:00",
  rangeEnd: "2026-04-08 00:03:22",
  bucketSeconds: 60,
  points: [
    {
      bucketStart: "2026-04-08 00:00:00",
      bucketEnd: "2026-04-08 00:00:59",
      totalCount: 3,
      successCount: 2,
      failureCount: 1,
      totalTokens: 120,
      totalCost: 0.5,
      nonSuccessCost: 0.2,
    },
    {
      bucketStart: "2026-04-08 00:02:00",
      bucketEnd: "2026-04-08 00:02:59",
      totalCount: 4,
      successCount: 4,
      failureCount: 0,
      totalTokens: 200,
      totalCost: 0.75,
      nonSuccessCost: 0,
    },
  ],
};
const tokenBreakdownResponse = {
  ...response,
  points: [
    {
      ...response.points[0],
      totalTokens: 120,
      inputTokens: 80,
      outputTokens: 40,
      cacheInputTokens: 50,
      reasoningTokens: 10,
    },
    {
      ...response.points[1],
      totalTokens: 200,
      inputTokens: 140,
      outputTokens: 60,
      cacheInputTokens: 100,
      reasoningTokens: 20,
    },
  ],
};
const invalidReasoningBreakdownResponse = {
  ...tokenBreakdownResponse,
  points: tokenBreakdownResponse.points.map((point, index) =>
    index === 0 ? { ...point, reasoningTokens: -1 } : point,
  ),
};
beforeAll(() => {
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: localStorageMock,
  });
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
  latestChartData = [];
  window.localStorage.clear();
  resetDashboardPerformanceDiagnostics();
});
function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}
function rerender(ui: React.ReactNode) {
  act(() => {
    root?.render(ui);
  });
}
function chartSection() {
  const section = host?.querySelector(
    '[data-testid="dashboard-today-activity-chart"]',
  ) as HTMLElement | null;
  if (!section) throw new Error("missing chart section");
  return section;
}
function interactionLayer() {
  const layer = host?.querySelector(
    '[data-testid="dashboard-today-activity-chart-interaction-layer"]',
  ) as HTMLElement | null;
  if (!layer) throw new Error("missing chart interaction layer");
  layer.getBoundingClientRect = () =>
    ({
      x: 0,
      y: 0,
      top: 0,
      right: 1000,
      bottom: 320,
      left: 0,
      width: 1000,
      height: 320,
      toJSON: () => ({}),
    }) as DOMRect;
  layer.setPointerCapture = vi.fn();
  layer.releasePointerCapture = vi.fn();
  layer.hasPointerCapture = vi.fn(() => true);
  return layer;
}
function dragLayer() {
  const layer = host?.querySelector(
    '[data-testid="dashboard-today-activity-chart-drag-layer"]',
  ) as HTMLElement | null;
  if (!layer) throw new Error("missing chart drag layer");
  return layer;
}
function setCompactViewport(matches: boolean) {
  const originalMatchMedia = window.matchMedia;
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: vi.fn(() => ({
      matches,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
    })),
  });
  return () => {
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: originalMatchMedia,
    });
  };
}
function dispatchWheel(element: HTMLElement, init: WheelEventInit & { clientX?: number }) {
  const event = new WheelEvent("wheel", {
    bubbles: true,
    cancelable: true,
    ...init,
  });
  if (init.clientX != null) {
    Object.defineProperty(event, "clientX", {
      configurable: true,
      value: init.clientX,
    });
  }
  act(() => {
    element.dispatchEvent(event);
  });
  return event;
}
function dispatchPointer(
  element: HTMLElement,
  type: string,
  init: MouseEventInit & { pointerId?: number },
) {
  const event = new MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    ...init,
  });
  Object.defineProperty(event, "pointerId", {
    configurable: true,
    value: init.pointerId ?? 1,
  });
  act(() => {
    element.dispatchEvent(event);
  });
}
async function flushAnimationFrame() {
  await act(async () => {
    await new Promise<void>((resolve) => {
      window.requestAnimationFrame(() => resolve());
    });
  });
}

export {
  chartSection,
  dispatchPointer,
  dispatchWheel,
  dragLayer,
  flushAnimationFrame,
  host,
  interactionLayer,
  invalidReasoningBreakdownResponse,
  latestChartData,
  localStorageMock,
  rechartsMock,
  render,
  rerender,
  response,
  root,
  setCompactViewport,
  storage,
  tokenBreakdownResponse,
};
