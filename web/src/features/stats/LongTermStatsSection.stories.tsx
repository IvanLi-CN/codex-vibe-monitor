import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { LongTermMetrics, LongTermStatsOverviewResponse } from "../../lib/api";
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
  ["model:gpt-5|reasoning:high", "gpt-5 · high", 128_400],
  ["model:gpt-5-mini|reasoning:low", "gpt-5-mini · low", 86_200],
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
  args: { overviewOverride: buildOverview(), seriesOverride: modelSeries },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    expect(canvas.getByTestId("long-term-stats-section")).toBeInTheDocument();
    await userEvent.click(canvas.getAllByRole("button", { name: "Cost" })[0]);
    await userEvent.type(canvas.getByPlaceholderText("Search names"), "gpt-5");
    expect(canvas.getByText("gpt-5 · high")).toBeInTheDocument();
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
