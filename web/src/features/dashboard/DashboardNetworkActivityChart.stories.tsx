import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, waitFor } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { DashboardNetworkTimeseriesResponse } from "../../lib/api";
import { DashboardNetworkActivityChart } from "./DashboardNetworkActivityChart";

const sampleResponse = {
  range: "today",
  rangeStart: "2026-07-16T10:50:00.000Z",
  rangeEnd: "2026-07-16T11:10:00.000Z",
  snapshotId: 1,
  bucketSeconds: 300,
  points: [
    {
      bucketStart: "2026-07-16T10:50:00.000Z",
      bucketEnd: "2026-07-16T10:55:00.000Z",
      uploadBytesPerSecond: 4_200,
      downloadBytesPerSecond: 18_400,
      uploadBytes: 1_140_000,
      downloadBytes: 5_400_000,
      isLiveBucket: false,
    },
    {
      bucketStart: "2026-07-16T10:55:00.000Z",
      bucketEnd: "2026-07-16T11:00:00.000Z",
      uploadBytesPerSecond: 0,
      downloadBytesPerSecond: 21.3 * 1024,
      uploadBytes: 1024,
      downloadBytes: Math.round(6.2 * 1024 * 1024),
      isLiveBucket: true,
    },
  ],
} satisfies DashboardNetworkTimeseriesResponse;

const meta = {
  title: "Dashboard/DashboardNetworkActivityChart",
  component: DashboardNetworkActivityChart,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  globals: {
    themeMode: "dark",
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div data-theme="vibe-dark" className="min-h-screen bg-[#08172b] px-8 py-8 text-white">
          <div className="mx-auto max-w-[1280px]">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof DashboardNetworkActivityChart>;

export default meta;

type Story = StoryObj<typeof meta>;

export const TooltipUploadDownload: Story = {
  args: {
    response: sampleResponse,
    loading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const chart = canvasElement.querySelector('[data-testid="dashboard-network-activity-chart"]');
    if (!(chart instanceof HTMLElement)) {
      throw new Error("missing network chart");
    }

    const rect = chart.getBoundingClientRect();
    const candidates = [
      [0.98, 0.3],
      [0.98, 0.45],
      [0.96, 0.3],
      [0.96, 0.45],
      [0.94, 0.45],
      [0.92, 0.5],
    ] as const;

    let matched = false;
    for (const [xf, yf] of candidates) {
      const clientX = rect.left + rect.width * xf;
      const clientY = rect.top + rect.height * yf;
      const target = document.elementFromPoint(clientX, clientY) ?? chart;
      target.dispatchEvent(
        new MouseEvent("mousemove", {
          bubbles: true,
          clientX,
          clientY,
        }),
      );
      try {
        await waitFor(
          () => {
            const tooltip = Array.from(document.querySelectorAll(".recharts-tooltip-wrapper")).find(
              (node) => {
                const text = node.textContent ?? "";
                return /Upload|上行/.test(text) && /Download|下行/.test(text);
              },
            );
            expect(tooltip?.textContent ?? "").toContain("1 KiB");
            expect(tooltip?.textContent ?? "").toContain("6.2 MiB");
          },
          { timeout: 300 },
        );
        matched = true;
        break;
      } catch {
        // Try the next hover coordinate until the live bucket tooltip is visible.
      }
    }

    if (!matched) {
      throw new Error("failed to reveal the upload/download tooltip");
    }
  },
};
