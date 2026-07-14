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
    avgFirstResponseByteTotalMs: 1290,
    usageDurationMs: 184200,
  },
  models: [
    {
      model: "gpt-5.6-sol",
      reasoningEffort: "high",
      tokensPerMinute: 1098,
      streamingResponseRate: 182.4,
      avgResponseMs: 5150,
      avgFirstResponseByteTotalMs: 1480,
      usageDurationMs: 118600,
    },
    {
      model: "gpt-5.6-terra",
      reasoningEffort: null,
      tokensPerMinute: 734,
      streamingResponseRate: null,
      avgResponseMs: null,
      avgFirstResponseByteTotalMs: 930,
      usageDurationMs: 65600,
    },
  ],
};

const meta = {
  title: "Dashboard/ModelPerformanceTrigger",
  component: ModelPerformanceTrigger,
  tags: ["autodocs"],
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
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: /open model performance/i }));
    const details = within(document.body).getByTestId("model-performance-tooltip-content");
    await expect(details).toBeVisible();
    await expect(details).toHaveTextContent("Model performance");
    await expect(within(details).getByRole("rowheader", { name: "Total" })).toBeInTheDocument();
    await expect(within(details).getByText("Unspecified")).toBeInTheDocument();
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
    await expect(within(document.body).getByRole("dialog")).toBeInTheDocument();
  },
};
