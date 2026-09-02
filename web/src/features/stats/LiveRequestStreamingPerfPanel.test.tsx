/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { LiveRequestStreamingEvaluation, LiveRequestStreamingPerf } from "../../lib/api";
import { LiveRequestStreamingPerfPanel } from "./LiveRequestStreamingPerfPanel";

vi.mock("../../i18n", () => ({
  useTranslation: () => ({
    t: (key: string, values?: Record<string, number | string>) => {
      const labels: Record<string, string> = {
        "stats.liveRequestStreaming.title": "Live request streaming",
        "stats.liveRequestStreaming.subtitle":
          "Compare buffered control with actual live-first traffic; treatment fallbacks are separate",
        "stats.liveRequestStreaming.control": "Buffered control",
        "stats.liveRequestStreaming.treatment": "Actual live-first treatment",
        "stats.liveRequestStreaming.treatmentFallback": "Treatment fallback: buffered",
        "stats.liveRequestStreaming.firstResponse": "P50 first response",
        "stats.liveRequestStreaming.firstToken": "P50 first token",
        "stats.liveRequestStreaming.overlap": "P50 upload overlap",
        "stats.liveRequestStreaming.retryRisk": "Fallback / retry",
        "stats.liveRequestStreaming.responseBenefit": "First-response benefit",
        "stats.liveRequestStreaming.tokenBenefit": "First-token benefit",
        "stats.liveRequestStreaming.overlapBenefit": "Overlap benefit",
        "stats.liveRequestStreaming.routeFinalization": "Route finalization",
        "stats.liveRequestStreaming.routeRawBytes": "Raw bytes at route (P50 / P90 / P99)",
        "stats.liveRequestStreaming.routeLogicalBytes": "Logical bytes at route (P50 / P90 / P99)",
        "stats.liveRequestStreaming.routeFinalizationMs": "P50 route finalization",
        "stats.liveRequestStreaming.routeEofBuffered": "Finalized at EOF",
        "stats.liveRequestStreaming.routeConservativeBuffered": "Conservative buffered",
        "stats.liveRequestStreaming.routeOutcomes": "Route outcomes",
        "stats.liveRequestStreaming.routeCacheHit": "Routing cache hit",
        "stats.liveRequestStreaming.routeColdLoad": "Routing cold load",
        "stats.liveRequestStreaming.evaluationTitle": "Decision from fixed 7-day window",
        "stats.liveRequestStreaming.evaluationStatus.keep": "Recommend keep",
        "stats.liveRequestStreaming.evaluationAssignments": "Treatment assignments",
        "stats.liveRequestStreaming.evaluationActualRate": "Actual live-first",
        "stats.liveRequestStreaming.evaluationFallbacks": "Buffered fallbacks",
        "stats.liveRequestStreaming.evaluationEvidence": "Evidence",
        "stats.liveRequestStreaming.evaluationMetric": "P50 benefit CI",
        "stats.liveRequestStreaming.evaluationRisk": "Risk upper bound",
        "stats.liveRequestStreaming.evaluationReasonCodes": "Reason codes",
      };
      if (key === "stats.liveRequestStreaming.insufficient") {
        return `Insufficient samples: ${values?.count} / ${values?.minimum}`;
      }
      if (key === "stats.liveRequestStreaming.coverage") {
        return `Coverage ${values?.rate} · ${values?.count} / ${values?.total}`;
      }
      if (key === "stats.liveRequestStreaming.metricCoverage") {
        return `Minimum metric samples: ${values?.count} / ${values?.minimum}`;
      }
      if (key === "stats.liveRequestStreaming.evaluationWindow") {
        return `Window ${values?.start} – ${values?.end}`;
      }
      if (key === "stats.liveRequestStreaming.noEffectiveTreatment") {
        return `No actual live-first samples. ${values?.count} treatment request(s) fell back to buffered, so benefits are unavailable.`;
      }
      if (key === "stats.liveRequestStreaming.treatmentFallbackNotice") {
        return `${values?.count} treatment request(s) fell back to buffered and are excluded from live-first benefits.`;
      }
      return labels[key] ?? key;
    },
  }),
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  host = null;
  root = null;
});

function render(data: LiveRequestStreamingPerf, evaluation?: LiveRequestStreamingEvaluation) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root?.render(<LiveRequestStreamingPerfPanel data={data} evaluation={evaluation} />));
}

const data: LiveRequestStreamingPerf = {
  coverage: 1,
  measuredInvocationCount: 400,
  responseInvocationCount: 400,
  cohorts: [
    {
      cohort: "control",
      transportMode: "buffered",
      successSampleCount: 200,
      invocationCount: 200,
      sufficientSamples: true,
      firstResponseByteTotalMs: { p50Ms: 1000, p90Ms: 1300, p99Ms: 1600 },
      firstTokenMs: { p50Ms: 1400, p90Ms: 1800, p99Ms: 2200 },
      requestUpstreamOverlapMs: { p50Ms: 0, p90Ms: 0, p99Ms: 0 },
      firstAttemptFailureRate: 0,
      fallbackOrRetryRate: 0.02,
      captureFailureRate: 0,
      ambiguousUpstreamDeliveryRate: 0,
    },
    {
      cohort: "treatment",
      transportMode: "live_first",
      successSampleCount: 200,
      invocationCount: 200,
      sufficientSamples: true,
      firstResponseByteTotalMs: { p50Ms: 800, p90Ms: 1000, p99Ms: 1400 },
      firstTokenMs: { p50Ms: 1100, p90Ms: 1500, p99Ms: 1900 },
      requestUpstreamOverlapMs: { p50Ms: 200, p90Ms: 350, p99Ms: 500 },
      firstAttemptFailureRate: 0,
      fallbackOrRetryRate: 0.03,
      captureFailureRate: 0,
      ambiguousUpstreamDeliveryRate: 0,
    },
  ],
  routeFinalization: {
    sampleCount: 200,
    sufficientSamples: true,
    rawBytes: { p50: 4096, p90: 8192, p99: 16384 },
    logicalBytes: { p50: 4000, p90: 8000, p99: 16000 },
    rawRatio: { p50: 1, p90: 1, p99: 1 },
    logicalRatio: { p50: 1, p90: 1, p99: 1 },
    finalizationMs: { p50Ms: 8, p90Ms: 12, p99Ms: 20 },
    eofFinalizedRate: 1,
    conservativeBufferedRate: 0.05,
    outcomeCounts: { live_first_model_ready: 200 },
    dependencyFactorCounts: { model: 200, prompt_cache: 40 },
    hotCacheHitRate: 0.995,
    coldLoadRate: 0.005,
  },
};

describe("LiveRequestStreamingPerfPanel", () => {
  it("shows absolute and relative p50 benefits for the two cohorts", () => {
    render(data);

    expect(host?.textContent).toContain("+200 ms (+20.0%)");
    expect(host?.textContent).toContain("+300 ms (+21.4%)");
    expect(host?.textContent).toContain("Overlap benefit+200 ms");
    expect(host?.textContent).toContain("4096 / 8192 / 16384 B");
    expect(host?.textContent).toContain("99.5%");
    expect(host?.textContent).toContain("live_first_model_ready: 200");
  });

  it("marks cohorts below the minimum successful sample count", () => {
    render({
      ...data,
      cohorts: data.cohorts.map((cohort) => ({
        ...cohort,
        successSampleCount: 8,
        sufficientSamples: false,
      })),
    });

    expect(host?.textContent).toContain("Minimum metric samples: 8 / 200");
    expect(host?.textContent).not.toContain("+200 ms (+20.0%)");
    expect(host?.textContent).not.toContain("+300 ms (+21.4%)");
  });

  it("keeps the invocation count visible when a metric has no samples", () => {
    render({
      ...data,
      cohorts: data.cohorts.map((cohort) =>
        cohort.cohort === "control"
          ? { ...cohort, requestUpstreamOverlapSampleCount: 0, sufficientSamples: false }
          : cohort,
      ),
    });

    const control = host?.querySelector('[data-testid="live-request-streaming-cohort-control"]');
    expect(control?.textContent).toContain("n=200");
    expect(control?.textContent).toContain("Minimum metric samples: 0 / 200");
  });

  it("does not compare a buffered treatment fallback as live-first", () => {
    render({
      ...data,
      cohorts: data.cohorts.map((cohort) =>
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
    });

    expect(host?.textContent).toContain("Treatment fallback: buffered");
    expect(host?.textContent).toContain(
      "No actual live-first samples. 200 treatment request(s) fell back to buffered, so benefits are unavailable.",
    );
    expect(host?.textContent).not.toContain("+200 ms (+20.0%)");
    expect(host?.textContent).not.toContain("+300 ms (+21.4%)");
  });

  it("renders the server-owned decision separately from diagnostic benefits", () => {
    render(data, {
      revision: "live-request-body-v2",
      endpoint: "/v1/responses",
      rangeStart: "2026-03-12T00:00:00Z",
      rangeEnd: "2026-03-19T00:00:00Z",
      treatmentAssignmentCount: 1200,
      treatmentEligibleCount: 1180,
      actualLiveFirstCount: 1150,
      treatmentBufferedFallbackCount: 50,
      actualLiveFirstRate: 1150 / 1200,
      cohorts: data.cohorts,
      routeFinalization: data.routeFinalization!,
      metrics: {
        firstResponse: { p50DifferenceMs: 150, lowerMs: 110, upperMs: 190 },
        firstToken: { p50DifferenceMs: 180, lowerMs: 120, upperMs: 240 },
        overlap: { p50DifferenceMs: 200, lowerMs: 80, upperMs: 320 },
      },
      risk: {
        firstAttemptFailure: { difference: 0, upperBound: 0.002 },
        fallbackOrRetry: { difference: 0, upperBound: 0.003 },
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
    });

    const evaluationPanel = host?.querySelector(
      '[data-testid="live-request-streaming-evaluation"]',
    );
    expect(evaluationPanel?.textContent).toContain("Recommend keep");
    expect(evaluationPanel?.textContent).toContain("Treatment assignments1200");
    expect(evaluationPanel?.textContent).toContain("latency_and_risk_thresholds_met");
  });
});
