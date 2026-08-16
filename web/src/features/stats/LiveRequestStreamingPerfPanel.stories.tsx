import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { LiveRequestStreamingPerf } from "../../lib/api";
import { LiveRequestStreamingPerfPanel } from "./LiveRequestStreamingPerfPanel";

const measured: LiveRequestStreamingPerf = {
  coverage: 0.98,
  measuredInvocationCount: 512,
  responseInvocationCount: 522,
  cohorts: [
    {
      cohort: "control",
      transportMode: "buffered",
      successSampleCount: 254,
      invocationCount: 260,
      sufficientSamples: true,
      firstResponseByteTotalMs: { p50Ms: 980, p90Ms: 1520, p99Ms: 2270 },
      firstTokenMs: { p50Ms: 1310, p90Ms: 2020, p99Ms: 2900 },
      requestUpstreamOverlapMs: { p50Ms: 0, p90Ms: 0, p99Ms: 0 },
      firstAttemptFailureRate: 0.012,
      fallbackOrRetryRate: 0.027,
      captureFailureRate: 0,
      ambiguousUpstreamDeliveryRate: 0,
    },
    {
      cohort: "treatment",
      transportMode: "live_first",
      successSampleCount: 250,
      invocationCount: 252,
      sufficientSamples: true,
      firstResponseByteTotalMs: { p50Ms: 760, p90Ms: 1190, p99Ms: 1780 },
      firstTokenMs: { p50Ms: 1030, p90Ms: 1620, p99Ms: 2340 },
      requestUpstreamOverlapMs: { p50Ms: 168, p90Ms: 340, p99Ms: 530 },
      firstAttemptFailureRate: 0.016,
      fallbackOrRetryRate: 0.032,
      captureFailureRate: 0.004,
      ambiguousUpstreamDeliveryRate: 0.004,
    },
  ],
};

const meta = {
  title: "Stats/LiveRequestStreamingPerfPanel",
  component: LiveRequestStreamingPerfPanel,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
          <div className="mx-auto w-full max-w-5xl">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof LiveRequestStreamingPerfPanel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Measured: Story = {
  args: { data: measured },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("live-request-streaming-perf-panel")).toBeVisible();
    await expect(canvas.getByText("Buffered control")).toBeVisible();
    await expect(canvas.getByText("Live-first treatment")).toBeVisible();
    await expect(canvas.getByText("+220 ms (+22.4%)")).toBeVisible();
  },
};

export const InsufficientSamples: Story = {
  args: {
    data: {
      ...measured,
      cohorts: measured.cohorts.map((cohort) => ({
        ...cohort,
        successSampleCount: 17,
        sufficientSamples: false,
      })),
    },
  },
};
