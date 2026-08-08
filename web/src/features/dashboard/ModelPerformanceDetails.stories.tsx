import type { Meta, StoryObj } from "@storybook/react-vite";
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
};

const meta = {
  title: "Dashboard/ModelPerformanceDetails",
  component: ModelPerformanceDetails,
  tags: ["autodocs"],
  parameters: { layout: "fullscreen" },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="min-h-screen bg-base-200 p-6 text-base-content sm:p-8">
          <div className="mx-auto w-full max-w-[72rem] rounded-xl border border-base-300 bg-base-100 p-4 shadow-sm sm:p-5">
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

export const Mobile390: Story = {
  args: { presentation: "drawer" },
  parameters: { viewport: { defaultViewport: "mobile390" } },
  render: (args) => (
    <div
      className="mx-auto w-[390px] max-w-full rounded-xl border border-base-300 bg-base-100 p-3 shadow-sm"
      data-testid="model-performance-mobile-390"
      style={{ width: "390px" }}
    >
      <ModelPerformanceDetails {...args} />
    </div>
  ),
};
