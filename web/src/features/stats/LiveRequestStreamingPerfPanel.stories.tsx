import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useEffect } from "react";
import { expect, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { LiveRequestStreamingEvaluation, LiveRequestStreamingPerf } from "../../lib/api";
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
  routeFinalization: {
    sampleCount: 250,
    sufficientSamples: true,
    rawBytes: { p50: 4820, p90: 12400, p99: 30500 },
    logicalBytes: { p50: 4750, p90: 12220, p99: 29900 },
    rawRatio: { p50: 1, p90: 1, p99: 1 },
    logicalRatio: { p50: 1, p90: 1, p99: 1 },
    finalizationMs: { p50Ms: 8, p90Ms: 22, p99Ms: 57 },
    eofFinalizedRate: 1,
    conservativeBufferedRate: 0.08,
    outcomeCounts: { live_first_model_ready: 250 },
    dependencyFactorCounts: {
      model: 250,
      sticky: 97,
      prompt_cache: 81,
      encrypted_owner: 12,
      image_capability: 26,
    },
    hotCacheHitRate: 0.992,
    coldLoadRate: 0.008,
  },
};
const measuredRouteFinalization = measured.routeFinalization!;
const evaluationKeep: LiveRequestStreamingEvaluation = {
  revision: "live-request-body-v2",
  endpoint: "/v1/responses",
  rangeStart: "2026-03-12T00:00:00Z",
  rangeEnd: "2026-03-19T00:00:00Z",
  treatmentAssignmentCount: 4200,
  treatmentEligibleCount: 4120,
  actualLiveFirstCount: 3980,
  treatmentBufferedFallbackCount: 220,
  actualLiveFirstRate: 3980 / 4200,
  cohorts: measured.cohorts,
  routeFinalization: measuredRouteFinalization,
  metrics: {
    firstResponse: { p50DifferenceMs: 220, lowerMs: 180, upperMs: 260 },
    firstToken: { p50DifferenceMs: 280, lowerMs: 230, upperMs: 330 },
    overlap: { p50DifferenceMs: 168, lowerMs: 120, upperMs: 220 },
  },
  risk: {
    firstAttemptFailure: { difference: 0.001, upperBound: 0.003 },
    fallbackOrRetry: { difference: 0.002, upperBound: 0.004 },
    captureFailure: { difference: 0, upperBound: 0.001 },
    ambiguousDelivery: { difference: 0, upperBound: 0.001 },
  },
  decision: {
    status: "recommend_keep",
    reasonCodes: ["latency_and_risk_thresholds_met"],
    minTreatmentAssignments: 1000,
    minActualLiveFirstRate: 0.05,
    minMetricSamples: 200,
    minLatencyBenefitMs: 100,
    maxRiskIncrease: 0.005,
    bootstrapResamples: 2000,
  },
};
const fallbackOnly: LiveRequestStreamingPerf = {
  ...measured,
  cohorts: measured.cohorts.map((cohort) =>
    cohort.cohort === "treatment"
      ? {
          ...cohort,
          transportMode: "buffered",
          sufficientSamples: false,
          requestUpstreamOverlapSampleCount: 0,
          requestUpstreamOverlapMs: null,
        }
      : cohort,
  ),
  routeFinalization: {
    ...measuredRouteFinalization,
    eofFinalizedRate: 0,
    conservativeBufferedRate: 1,
    outcomeCounts: { buffered_no_model: 252 },
  },
};

function ThemeRoot({ children }: { children: ReactNode }) {
  useEffect(() => {
    const previousTheme = document.documentElement.getAttribute("data-theme");
    document.documentElement.setAttribute("data-theme", "vibe-dark");
    return () => {
      if (previousTheme) {
        document.documentElement.setAttribute("data-theme", previousTheme);
      } else {
        document.documentElement.removeAttribute("data-theme");
      }
    };
  }, []);

  return <div data-theme="vibe-dark">{children}</div>;
}

const meta = {
  title: "Stats/LiveRequestStreamingPerfPanel",
  component: LiveRequestStreamingPerfPanel,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    viewport: { defaultViewport: "desktop1280" },
  },
  decorators: [
    (Story) => (
      <ThemeRoot>
        <I18nProvider>
          <div
            data-visual-evidence-surface
            className="min-h-screen bg-base-200 px-6 py-6 text-base-content"
          >
            <div data-visual-evidence-target className="mx-auto w-full max-w-5xl">
              <Story />
            </div>
          </div>
        </I18nProvider>
      </ThemeRoot>
    ),
  ],
} satisfies Meta<typeof LiveRequestStreamingPerfPanel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Measured: Story = {
  tags: ["test"],
  args: { data: measured, evaluation: evaluationKeep },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("live-request-streaming-perf-panel")).toBeVisible();
    await expect(canvas.getByText("缓冲对照组")).toBeVisible();
    await expect(canvas.getByText("实际实时首发实验组")).toBeVisible();
    await expect(canvas.getByText("+220 ms (+22.4%)")).toBeVisible();
    await expect(canvas.getByText("+168 ms")).toBeVisible();
    await expect(canvas.getByText("固定 7 天窗口结论")).toBeVisible();
    await expect(canvas.getByText("建议保留")).toBeVisible();
  },
};

export const DecisionReview: Story = {
  tags: ["test"],
  args: {
    data: measured,
    evaluation: {
      ...evaluationKeep,
      decision: {
        ...evaluationKeep.decision,
        status: "review_required",
        reasonCodes: ["latency_or_risk_evidence_inconclusive", "risk_increase_above_threshold"],
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("需要人工审查")).toBeVisible();
    await expect(canvas.getByText(/risk_increase_above_threshold/)).toBeVisible();
  },
};

export const InsufficientSamples: Story = {
  tags: ["test"],
  args: {
    data: {
      ...measured,
      cohorts: measured.cohorts.map((cohort) => ({
        ...cohort,
        successSampleCount: 17,
        sufficientSamples: false,
      })),
      routeFinalization: {
        ...measuredRouteFinalization,
        sampleCount: 17,
        sufficientSamples: false,
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getAllByText("最低指标样本：17 / 200")).toHaveLength(2);
    await expect(canvas.getByText("样本不足：17 / 200")).toBeVisible();
    await expect(canvas.queryByText("+220 ms (+22.4%)")).not.toBeInTheDocument();
  },
};

export const FallbackOnly: Story = {
  tags: ["test"],
  args: { data: fallbackOnly },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(
      canvas.getByText("尚无实际实时首发样本。252 个实验组请求已回退为缓冲转发，收益不可比较。"),
    ).toBeVisible();
    await expect(canvas.getByText("实验组回退：缓冲转发")).toBeVisible();
    await expect(canvas.queryByText("+220 ms (+22.4%)")).not.toBeInTheDocument();
  },
};
