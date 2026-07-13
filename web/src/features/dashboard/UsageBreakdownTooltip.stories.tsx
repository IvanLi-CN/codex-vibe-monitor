import type { Meta, StoryObj } from "@storybook/react-vite";
import type { UsageBreakdown } from "../../lib/api";
import { UsageBreakdownTooltip } from "./UsageBreakdownTooltip";

const labels = {
  total: "Total",
  model: "Model",
  cacheWrite: "Cache write",
  cacheRead: "Cache read",
  cacheHitRate: "Cache hit rate",
  output: "Output",
  unknownModel: "Unidentified model",
  reasoningEffort: "Reasoning effort",
  unspecifiedEffort: "Unspecified",
  effortNone: "None",
  effortMinimal: "Minimal",
  effortLow: "Low",
  effortMedium: "Medium",
  effortHigh: "High",
  effortXhigh: "XHigh",
};

const exactBreakdown: UsageBreakdown = {
  cacheWriteTokens: 432_000,
  cacheReadTokens: 196_000,
  outputTokens: 214_190,
  costs: {
    input: 1.96,
    cacheWrite: 3.24,
    cacheRead: 0.44,
    output: 6.12,
    reasoning: 0.71,
    unknown: 0,
  },
  models: [
    {
      model: "gpt-5.6",
      reasoningEffort: "high",
      cacheWriteTokens: 290_000,
      cacheReadTokens: 128_000,
      outputTokens: 146_120,
      costs: {
        input: 1.24,
        cacheWrite: 2.11,
        cacheRead: 0.29,
        output: 4.13,
        reasoning: 0.49,
        unknown: 0,
      },
    },
    {
      model: "gpt-5.4-mini",
      cacheWriteTokens: 142_000,
      cacheReadTokens: 68_000,
      outputTokens: 68_070,
      costs: {
        input: 0.72,
        cacheWrite: 1.13,
        cacheRead: 0.15,
        output: 1.99,
        reasoning: 0.22,
        unknown: 0,
      },
    },
  ],
};

const historicalBreakdown: UsageBreakdown = {
  ...exactBreakdown,
  costs: { input: 0, cacheWrite: 0, cacheRead: 0, output: 0, reasoning: 0, unknown: 12.47 },
  models: exactBreakdown.models.map((model) => ({
    ...model,
    costs: {
      input: 0,
      cacheWrite: 0,
      cacheRead: 0,
      output: 0,
      reasoning: 0,
      unknown: model.model === "gpt-5.6" ? 9.48 : 2.99,
    },
  })),
};

const missingCostBreakdown: UsageBreakdown = {
  cacheWriteTokens: exactBreakdown.cacheWriteTokens,
  cacheReadTokens: exactBreakdown.cacheReadTokens,
  outputTokens: exactBreakdown.outputTokens,
  models: exactBreakdown.models.map(({ costs: _costs, ...model }) => model),
};

function formatNumber(value: number) {
  return new Intl.NumberFormat("en-US").format(value);
}

function formatCurrency(value: number) {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 4,
  }).format(value);
}

function formatRatio(value: number | null) {
  return value == null
    ? "—"
    : new Intl.NumberFormat("en-US", { style: "percent", maximumFractionDigits: 1 }).format(value);
}

const meta = {
  title: "Dashboard/UsageBreakdownTooltip",
  component: UsageBreakdownTooltip,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  args: {
    title: "Usage details",
    breakdown: exactBreakdown,
    formatNumber,
    formatRatio,
    formatCurrency,
    labels,
  },
  render: (args) => (
    <div className="min-h-screen bg-base-200 px-4 py-6 text-base-content sm:px-6">
      <div className="mx-auto w-full max-w-[48rem] rounded-xl border border-base-300 bg-base-100 p-3 shadow-sm sm:p-4">
        <UsageBreakdownTooltip {...args} />
      </div>
    </div>
  ),
} satisfies Meta<typeof UsageBreakdownTooltip>;

export default meta;

type Story = StoryObj<typeof meta>;

export const ExactCosts: Story = {
  args: { breakdown: exactBreakdown },
};

export const HistoricalTotalOnly: Story = {
  args: { breakdown: historicalBreakdown },
};

export const MissingCostDetails: Story = {
  args: { breakdown: missingCostBreakdown },
};

export const Mobile390: Story = {
  ...ExactCosts,
  globals: {
    viewport: { value: "mobile390", isRotated: false },
  },
};
