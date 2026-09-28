import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { ModelPerformance } from "../../lib/api";
import { ModelPerformanceDetails } from "./ModelPerformanceDetails";

const performance: ModelPerformance = {
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
      reasoningEffort: " MAX ",
      tokensPerMinute: 1098,
      streamingResponseRate: 182.4,
      avgResponseMs: 5150,
      avgFirstTokenMs: 1480,
      wallClockUsageDurationMs: 86400,
      cumulativeUsageDurationMs: 118600,
      parallelism: 1.37,
    },
    {
      model: "gpt-5.6-sol",
      reasoningEffort: "low",
      tokensPerMinute: 514,
      streamingResponseRate: 151.3,
      avgResponseMs: 4680,
      avgFirstTokenMs: 1110,
      wallClockUsageDurationMs: 22000,
      cumulativeUsageDurationMs: 24800,
      parallelism: 1.13,
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
      reasoningEffort: "ULTRA",
      tokensPerMinute: 648,
      streamingResponseRate: 141.8,
      avgResponseMs: 4380,
      avgFirstTokenMs: 880,
      wallClockUsageDurationMs: 46200,
      cumulativeUsageDurationMs: 58900,
      parallelism: 1.27,
    },
  ],
  modelGroups: [
    {
      model: "gpt-5.6-sol",
      reasoningEffort: null,
      tokensPerMinute: 1612,
      streamingResponseRate: 177.1,
      avgResponseMs: 5060,
      avgFirstTokenMs: 1410,
      wallClockUsageDurationMs: 86400,
      cumulativeUsageDurationMs: 143400,
      parallelism: 1.66,
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
      reasoningEffort: null,
      tokensPerMinute: 648,
      streamingResponseRate: 141.8,
      avgResponseMs: 4380,
      avgFirstTokenMs: 880,
      wallClockUsageDurationMs: 46200,
      cumulativeUsageDurationMs: 58900,
      parallelism: 1.27,
    },
  ],
};

const meta = {
  title: "Dashboard/ModelPerformanceDetails",
  component: ModelPerformanceDetails,
  tags: ["autodocs"],
  parameters: { layout: "fullscreen" },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div
          data-visual-evidence-surface="dashboard-model-performance-details"
          className="min-h-screen bg-base-200 p-6 text-base-content sm:p-8"
        >
          <div
            data-visual-evidence-target="dashboard-model-performance-details-table"
            className="mx-auto w-full max-w-[72rem]"
          >
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
  args: {
    title: "Model performance",
    performance,
    presentation: "tooltip",
  },
} satisfies Meta<typeof ModelPerformanceDetails>;

export default meta;

type Story = StoryObj<typeof meta>;

export const ReasoningAndIdentityMatrix: Story = {};

export const SimpleGrouping: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByTestId("dashboard-model-breakdown-mode-simple"));
    await expect(canvas.getByTestId("dashboard-model-breakdown-mode-simple")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(
      canvas.getByTestId("dashboard-model-performance-table-model-context"),
    ).toHaveLength(3);
  },
};

export const Mobile390: Story = {
  args: { presentation: "drawer" },
  parameters: { viewport: { defaultViewport: "mobile390" } },
  render: (args) => (
    <div
      data-visual-evidence-surface="dashboard-model-performance-details-mobile"
      className="min-h-screen w-[390px] max-w-full bg-base-200 p-4 text-base-content"
      data-testid="model-performance-mobile-390"
    >
      <div data-visual-evidence-target="dashboard-model-performance-details-mobile-content">
        <ModelPerformanceDetails {...args} />
      </div>
    </div>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvasElement.querySelector("select")).toBeNull();
    await expect(canvas.getByTestId("dashboard-model-breakdown-model-sort")).toBeInTheDocument();
    await expect(canvas.getByTestId("dashboard-model-breakdown-sort-tpm")).toBeInTheDocument();
    await userEvent.click(canvas.getByTestId("dashboard-model-breakdown-model-sort"));
    await expect(canvas.getByTestId("dashboard-model-breakdown-model-sort")).toHaveAttribute(
      "aria-label",
      expect.stringMatching(/name/i),
    );
  },
};
