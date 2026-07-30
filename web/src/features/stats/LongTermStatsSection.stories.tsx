import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { LongTermMetrics, LongTermSeries, LongTermStatsOverviewResponse } from "../../lib/api";
import { LongTermStatsSection } from "./LongTermStatsSection";

const metrics = (tokens: number, cost: number, calls: number): LongTermMetrics => ({
  calls,
  tokens,
  tokenSamples: calls,
  cost,
  costSamples: calls,
  usageTimeMs: calls * 900,
  usageTimeSamples: calls,
  wallTimeMs: calls * 500,
  wallTimeSamples: calls,
  outputSpeedTokensPerSecond: 42,
  outputSpeedSamples: calls,
  firstByteMs: 320,
  firstByteSamples: calls,
  responseMs: 1400,
  responseSamples: calls,
});

const dates = Array.from({ length: 7 }, (_, index) => {
  const date = new Date(Date.UTC(2026, 6, 20 + index));
  return date.toISOString().slice(0, 10);
});

const modelEntries = [
  ["model:gpt-5|reasoning:high", "gpt-5", 128_400],
  ["model:gpt-5-mini|reasoning:low", "gpt-5-mini", 86_200],
  [
    "model:very-long-model-name-for-table-truncation|reasoning:medium",
    "very-long-model-name-for-table-truncation",
    22_700,
  ],
] as const;

const buildOverview = (
  status: LongTermStatsOverviewResponse["status"] = "ready",
): LongTermStatsOverviewResponse => ({
  status,
  statisticsStartDate: "2026-01-01",
  processedRows: status === "preparing" ? 180 : 412,
  totalRows: 412,
  timezone: "Asia/Shanghai",
  range: "7d",
  global: metrics(237_300, 18.72, 412),
  daily: dates.map((date, index) => ({
    date,
    ...metrics(24_000 + index * 1_200, 1.4 + index / 10, 34 + index),
  })),
  models: modelEntries.map(([seriesKey, displayName, tokens], index) => ({
    seriesKey,
    displayName,
    reasoningEffort: index === 0 ? "high" : index === 1 ? "low" : "medium",
    ...metrics(tokens, tokens / 13_000, 90 - index * 18),
  })),
  upstreams: [
    {
      seriesKey: "account:11",
      displayName: "Production key · west",
      ...metrics(151_000, 10.4, 241),
    },
    { seriesKey: "account:12", displayName: "Research key · east", ...metrics(71_000, 5.1, 118) },
    { seriesKey: "other", displayName: "其他", ...metrics(15_300, 3.2, 53) },
  ],
});

const modelSeries = modelEntries.map(([seriesKey, displayName]) => ({
  seriesKey,
  displayName,
  points: dates.map((date, index) => ({
    date,
    ...metrics(12_000 + index * 600, 0.9 + index / 20, 20 + index),
  })),
}));

const upstreamSeries: LongTermSeries[] = [
  ["account:11", "Production key · west", 151_000] as const,
  ["account:12", "Research key · east", 71_000] as const,
  ["other", "其他", 15_300] as const,
].map(([seriesKey, displayName, tokens]) => ({
  seriesKey,
  displayName,
  points: dates.map((date, index) => ({
    date,
    ...metrics(Number(tokens) / 7 + index * 300, 1.2 + index / 10, 16 + index),
  })),
}));

const sparseDates = Array.from({ length: 21 }, (_, index) => {
  const date = new Date(Date.UTC(2026, 6, 10 + index));
  return date.toISOString().slice(0, 10);
});

const sparseOverview: LongTermStatsOverviewResponse = {
  ...buildOverview(),
  range: "30d",
  daily: sparseDates.map((date, index) => ({
    date,
    ...metrics(18_000 + index * 700, 1.1 + index / 12, 26 + index),
  })),
};

const sparseIndices = new Set([0, 3, 15, 20]);
const sparseModelSeries: LongTermSeries[] = modelEntries.map(
  ([seriesKey, displayName], seriesIndex) => ({
    seriesKey,
    displayName,
    points: sparseDates
      .filter((_, index) => sparseIndices.has(index))
      .map((date, index) => ({
        date,
        ...metrics(9_000 + seriesIndex * 3_200 + index * 800, 0.7 + index / 10, 12 + index),
      })),
  }),
);

const sparseUpstreamSeries: LongTermSeries[] = upstreamSeries.map((entry, seriesIndex) => ({
  ...entry,
  points: sparseDates
    .filter((_, index) => sparseIndices.has(index))
    .map((date, index) => ({
      date,
      ...metrics(11_000 + seriesIndex * 2_500 + index * 600, 0.8 + index / 10, 14 + index),
    })),
}));

const meta = {
  title: "Stats/LongTermStatsSection",
  component: LongTermStatsSection,
  decorators: [
    (Story) => (
      <I18nProvider>
        <Story />
      </I18nProvider>
    ),
  ],
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof LongTermStatsSection>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Ready: Story = {
  args: {
    overviewOverride: buildOverview(),
    seriesOverride: modelSeries,
    upstreamSeriesOverride: upstreamSeries,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    expect(canvas.getByTestId("long-term-stats-section")).toBeInTheDocument();
    await userEvent.click(canvas.getAllByRole("button", { name: "Cost" })[0]);
    const modelUsageChart = within(canvas.getByTestId("long-term-chart-model-usage"));
    const upstreamUsageChart = within(canvas.getByTestId("long-term-chart-upstream-usage"));
    await userEvent.click(modelUsageChart.getByRole("button", { name: "Calls" }));
    await userEvent.click(upstreamUsageChart.getByRole("button", { name: "Cost" }));
    expect(
      canvas
        .getByTestId("long-term-chart-model-usage")
        .querySelector('[data-chart-mode="stacked-area"]'),
    ).toBeTruthy();
    expect(
      canvas
        .getByTestId("long-term-chart-upstream-usage")
        .querySelector('[data-chart-mode="stacked-area"]'),
    ).toBeTruthy();
    await userEvent.type(canvas.getByPlaceholderText("Search names"), "gpt-5");
    expect(canvas.getAllByText("gpt-5").length).toBeGreaterThan(0);
    expect(canvas.getByTestId("long-term-model-total-row")).toBeInTheDocument();
  },
};

export const SparseSeries: Story = {
  args: {
    initialRange: "30d",
    overviewOverride: sparseOverview,
    seriesOverride: sparseModelSeries,
    upstreamSeriesOverride: sparseUpstreamSeries,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    expect(canvas.getByTestId("long-term-chart-model-usage")).toBeInTheDocument();
    expect(canvas.getByTestId("long-term-chart-upstream-usage")).toBeInTheDocument();
    expect(
      canvas
        .getByTestId("long-term-chart-model-usage")
        .querySelector('[data-chart-mode="stacked-area"]'),
    ).toBeTruthy();
    expect(
      canvas
        .getByTestId("long-term-chart-upstream-usage")
        .querySelector('[data-chart-mode="stacked-area"]'),
    ).toBeTruthy();
  },
};

export const Preparing: Story = {
  args: { overviewOverride: buildOverview("preparing") },
};

export const Empty: Story = {
  args: {
    overviewOverride: {
      ...buildOverview("empty"),
      models: [],
      upstreams: [],
      daily: [],
      global: metrics(0, 0, 0),
    },
  },
};

export const ErrorState: Story = {
  args: { overviewOverride: buildOverview("error") },
};
