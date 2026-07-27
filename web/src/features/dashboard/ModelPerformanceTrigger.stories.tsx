import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { ModelPerformance } from "../../lib/api";
import { ModelPerformanceTrigger } from "./ModelPerformanceTrigger";

const modelPerformance: ModelPerformance = {
  available: true,
  total: {
    tokensPerMinute: 1832,
    streamingResponseRate: 164.2,
    avgResponseMs: 4820,
    avgFirstTokenMs: 1290,
    wallClockUsageDurationMs: 101200,
    cumulativeUsageDurationMs: 132400,
    parallelism: 1.31,
  },
  models: [
    {
      model: "gpt-5.6-sol",
      reasoningEffort: "high",
      tokensPerMinute: 1098,
      streamingResponseRate: 182.4,
      avgResponseMs: 5150,
      avgFirstTokenMs: 1480,
      wallClockUsageDurationMs: 86400,
      cumulativeUsageDurationMs: 118600,
      parallelism: 1.37,
    },
    {
      model: "gpt-5.6-terra",
      reasoningEffort: null,
      tokensPerMinute: 734,
      streamingResponseRate: null,
      avgResponseMs: null,
      avgFirstTokenMs: 930,
      wallClockUsageDurationMs: 50800,
      cumulativeUsageDurationMs: 65600,
      parallelism: 1.29,
    },
    {
      model: "gpt-5.6-luna",
      reasoningEffort: "medium",
      tokensPerMinute: 648,
      streamingResponseRate: 141.8,
      avgResponseMs: 4380,
      avgFirstTokenMs: 880,
      wallClockUsageDurationMs: 46200,
      cumulativeUsageDurationMs: 58900,
      parallelism: 1.27,
    },
    {
      model: "gpt-5.6-sol-2026-07-27",
      reasoningEffort: "low",
      tokensPerMinute: 592,
      streamingResponseRate: 128.6,
      avgResponseMs: 3950,
      avgFirstTokenMs: 760,
      wallClockUsageDurationMs: 41100,
      cumulativeUsageDurationMs: 48700,
      parallelism: 1.18,
    },
    {
      model: "gpt-5.6-terra-experimental-routing-variant-with-a-very-long-name",
      reasoningEffort: "adaptive-experimental",
      tokensPerMinute: 436,
      streamingResponseRate: 103.4,
      avgResponseMs: 5260,
      avgFirstTokenMs: 1130,
      wallClockUsageDurationMs: 37800,
      cumulativeUsageDurationMs: 42900,
      parallelism: 1.13,
    },
    {
      model: "gpt-5.6-luna-2026-07-27",
      reasoningEffort: "minimal",
      tokensPerMinute: 384,
      streamingResponseRate: 96.2,
      avgResponseMs: 3410,
      avgFirstTokenMs: 690,
      wallClockUsageDurationMs: 32200,
      cumulativeUsageDurationMs: 36500,
      parallelism: 1.13,
    },
    {
      model: "gpt-5.5-codex",
      reasoningEffort: null,
      tokensPerMinute: 305,
      streamingResponseRate: null,
      avgResponseMs: 6120,
      avgFirstTokenMs: null,
      wallClockUsageDurationMs: 28600,
      cumulativeUsageDurationMs: 31700,
      parallelism: 1.11,
    },
  ],
};

const meta = {
  title: "Dashboard/ModelPerformanceTrigger",
  component: ModelPerformanceTrigger,
  tags: ["autodocs"],
  parameters: {
    docs: {
      description: {
        component:
          "Responsive model-performance details with a dense desktop table and compact mobile drawer.",
      },
    },
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="min-h-screen bg-base-200 p-8 text-base-content">
          <Story />
        </div>
      </I18nProvider>
    ),
  ],
  args: {
    title: "Model performance",
    ariaLabel: "Open model performance details",
    performance: modelPerformance,
    children: (
      <span className="inline-flex cursor-pointer rounded-md border border-primary/40 bg-primary/10 px-3 py-2 font-mono font-semibold text-primary">
        1,832 TPM
      </span>
    ),
  },
} satisfies Meta<typeof ModelPerformanceTrigger>;

export default meta;

type Story = StoryObj<typeof meta>;

export const DesktopTooltip: Story = {
  parameters: {
    viewport: { defaultViewport: "desktop1440" },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: /open model performance/i }));
    const details = within(document.body)
      .getAllByTestId("model-performance-tooltip-content")
      .find((element) => element.getBoundingClientRect().width > 1);
    await expect(details).toBeDefined();
    if (!details) return;
    await expect(details).toBeVisible();
    await expect(details).toHaveTextContent("Model performance");
    await expect(within(details).getByRole("rowheader", { name: "Total" })).toBeInTheDocument();
    await expect(within(details).getByText("Unspecified")).toBeInTheDocument();
    await expect(details).toHaveTextContent("Wall clock");
    await expect(details).toHaveTextContent("Cumulative");
    await expect(details).toHaveTextContent("x1.31");
    const scrollRegion = within(details).getByTestId("model-performance-table-scroll-region");
    await expect(scrollRegion.scrollWidth).toBeLessThanOrEqual(scrollRegion.clientWidth);
    const modelRows = within(details).getAllByTestId("model-performance-table-model-context");
    await expect(modelRows).toHaveLength(7);
    await expect(
      modelRows.filter((row) => row.dataset.modelContextDisplay === "model-badge"),
    ).toHaveLength(5);
    await expect(
      modelRows.filter((row) => row.dataset.modelContextDisplay === "name-and-effort"),
    ).toHaveLength(2);
    for (const row of modelRows) {
      await expect(getComputedStyle(row).whiteSpace).toBe("nowrap");
      await expect(row.getBoundingClientRect().height).toBeLessThanOrEqual(32);
    }
    const metricCells = within(details).getAllByRole("cell");
    await expect(metricCells.length).toBeGreaterThan(0);
    for (const cell of metricCells) {
      await expect(getComputedStyle(cell).overflowX).toBe("hidden");
    }
  },
};

export const Empty: Story = {
  args: {
    performance: {
      available: true,
      total: { tokensPerMinute: 0 },
      models: [],
    },
  },
};

export const Unavailable: Story = {
  args: {
    performance: {
      available: false,
      total: { tokensPerMinute: 0 },
      models: [],
    },
  },
};

export const MobileDrawer: Story = {
  parameters: {
    viewport: { defaultViewport: "mobile390" },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: /open model performance/i }));
    const dialog = within(document.body).getByRole("dialog");
    await expect(dialog).toBeInTheDocument();
    await expect(dialog).toHaveTextContent("x1.31");
    await expect(dialog.scrollWidth).toBeLessThanOrEqual(dialog.clientWidth);
    await expect(
      within(dialog).getAllByTestId("model-performance-drawer-model-context"),
    ).toHaveLength(7);
  },
};
