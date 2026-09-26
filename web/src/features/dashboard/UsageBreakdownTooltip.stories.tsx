import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
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
      reasoningEffort: " MAX ",
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
      model: "gpt-5.6",
      reasoningEffort: "medium",
      cacheWriteTokens: 60_000,
      cacheReadTokens: 28_000,
      outputTokens: 30_000,
      costs: {
        input: 0.26,
        cacheWrite: 0.44,
        cacheRead: 0.06,
        output: 0.85,
        reasoning: 0.1,
        unknown: 0,
      },
    },
    {
      model: "gpt-5.6-luna-2026-07-27",
      reasoningEffort: "ULTRA",
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
      unknown: model.model === "gpt-5.6" ? (model.reasoningEffort === " MAX " ? 5.76 : 3.72) : 2.99,
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
  decorators: [
    (Story) => (
      <I18nProvider>
        <Story />
      </I18nProvider>
    ),
  ],
  args: {
    title: "Usage details",
    breakdown: exactBreakdown,
    formatNumber,
    formatRatio,
    formatCurrency,
    labels,
  },
  render: (args) => (
    <div
      data-visual-evidence-surface="dashboard-usage-breakdown-tooltip"
      className="min-h-screen bg-base-200 px-4 py-6 text-base-content sm:px-6"
    >
      <div
        data-visual-evidence-target="dashboard-usage-breakdown-table"
        className="mx-auto w-full max-w-[48rem]"
      >
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

export const SimpleGrouping: Story = {
  args: { breakdown: exactBreakdown },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByTestId("dashboard-model-breakdown-mode-simple"));
    await expect(canvas.getByTestId("dashboard-model-breakdown-mode-simple")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(canvas.getAllByRole("row")).toHaveLength(4);
    await expect(canvasElement.querySelector("select")).toBeNull();
    await expect(
      canvasElement.querySelector('[data-testid="dashboard-model-breakdown-sort-cache-write"]'),
    ).toBeNull();
    await expect(
      canvasElement.querySelector('[data-testid="dashboard-model-breakdown-sort-cache-read"]'),
    ).toBeNull();
    await expect(
      canvasElement.querySelector('[data-testid="dashboard-model-breakdown-sort-output"]'),
    ).toBeNull();
    await expect(canvas.getByTestId("dashboard-model-breakdown-sort-total")).toBeInTheDocument();
  },
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
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("usage-breakdown-mobile-list")).toBeInTheDocument();
    await expect(canvas.getByTestId("usage-breakdown-mobile-controls")).toBeInTheDocument();
    await expect(canvas.getByTestId("dashboard-model-breakdown-sort-total")).toBeInTheDocument();
    await expect(canvas.getByTestId("usage-breakdown-table-scroll-region")).toHaveClass("hidden");
  },
};
