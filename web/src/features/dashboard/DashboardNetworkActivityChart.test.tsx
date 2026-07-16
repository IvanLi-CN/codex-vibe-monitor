/** @vitest-environment jsdom */
import type { ReactNode } from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { DashboardNetworkTimeseriesResponse } from "../../lib/api";
import {
  buildDashboardNetworkTooltipRows,
  DashboardNetworkActivityChart,
} from "./DashboardNetworkActivityChart";

type CapturedTooltipProps = {
  content?: (props: {
    active?: boolean;
    payload?: Array<{
      dataKey?: string;
      value?: number | string;
      payload?: Record<string, unknown>;
    }>;
  }) => ReactNode;
};

let latestTooltipProps: CapturedTooltipProps | null = null;

vi.mock("recharts", () => ({
  ResponsiveContainer: ({ children }: { children: ReactNode }) => (
    <div data-testid="responsive">{children}</div>
  ),
  AreaChart: ({ children }: { children: ReactNode }) => (
    <div data-testid="area-chart">{children}</div>
  ),
  CartesianGrid: () => <div data-testid="grid" />,
  XAxis: () => <div data-testid="x-axis" />,
  YAxis: () => <div data-testid="y-axis" />,
  Legend: () => <div data-testid="legend" />,
  Area: () => <div data-testid="area" />,
  Tooltip: (props: CapturedTooltipProps) => {
    latestTooltipProps = props;
    return <div data-testid="tooltip" />;
  },
}));

vi.mock("../../i18n", () => ({
  useTranslation: () => ({
    locale: "en",
    t: (key: string) => {
      const map: Record<string, string> = {
        "chart.loadingDetailed": "Loading chart",
        "chart.noDataRange": "No data",
        "dashboard.activityOverview.networkUpload": "Upload",
        "dashboard.activityOverview.networkDownload": "Download",
        "dashboard.activityOverview.networkLiveNote": "5-minute average",
        "dashboard.activityOverview.networkRefreshing": "Refreshing",
      };
      return map[key] ?? key;
    },
  }),
}));

vi.mock("../../theme", () => ({
  useTheme: () => ({
    themeMode: "dark",
  }),
}));

const response = {
  range: "today",
  rangeStart: "2026-07-16T10:50:00.000Z",
  rangeEnd: "2026-07-16T10:55:00.000Z",
  snapshotId: 1,
  bucketSeconds: 300,
  points: [
    {
      bucketStart: "2026-07-16T10:50:00.000Z",
      bucketEnd: "2026-07-16T10:55:00.000Z",
      uploadBytesPerSecond: 0,
      downloadBytesPerSecond: 21.3 * 1024,
      uploadBytes: 1024,
      downloadBytes: Math.round(6.2 * 1024 * 1024),
      isLiveBucket: true,
    },
  ],
} satisfies DashboardNetworkTimeseriesResponse;

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function renderChart() {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <DashboardNetworkActivityChart response={response} loading={false} error={null} />,
    );
  });
}

afterEach(() => {
  latestTooltipProps = null;
  act(() => {
    root?.unmount();
  });
  root = null;
  host?.remove();
  host = null;
});

describe("DashboardNetworkActivityChart", () => {
  it("renders a dark-theme tooltip surface with upload and download icons", () => {
    renderChart();

    const rows = buildDashboardNetworkTooltipRows(
      [
        {
          dataKey: "uploadBytesPerSecond",
          value: 0,
          payload: response.points[0],
        },
        {
          dataKey: "downloadBytesPerSecond",
          value: response.points[0].downloadBytesPerSecond,
          payload: response.points[0],
        },
      ],
      "en-US",
      (key) =>
        ({
          "dashboard.activityOverview.networkUpload": "Upload",
          "dashboard.activityOverview.networkDownload": "Download",
        })[key] ?? key,
    );

    expect(rows).toEqual([
      expect.objectContaining({
        iconName: "arrow-up-bold",
        label: "Upload",
        value: expect.stringContaining("0 B/s"),
      }),
      expect.objectContaining({
        iconName: "arrow-down-bold",
        label: "Download",
        value: expect.stringContaining("21.3 KiB/s"),
      }),
    ]);
    expect(rows[0]?.value).toContain("1 KiB");
    expect(rows[1]?.value).toContain("6.2 MiB");

    const tooltipNode = latestTooltipProps?.content?.({
      active: true,
      payload: [
        {
          dataKey: "uploadBytesPerSecond",
          value: 0,
          payload: response.points[0],
        },
        {
          dataKey: "downloadBytesPerSecond",
          value: response.points[0].downloadBytesPerSecond,
          payload: response.points[0],
        },
      ],
    });
    const html = renderToStaticMarkup(<>{tooltipNode}</>);

    expect(
      host
        ?.querySelector('[data-testid="dashboard-network-activity-chart"]')
        ?.getAttribute("data-theme"),
    ).toBe("vibe-dark");
    expect(html).toContain('data-theme="vibe-dark"');
    expect(html).toContain("Upload");
    expect(html).toContain("Download");
    expect((html.match(/data-icon=/g) ?? []).length).toBe(2);
  });
});
