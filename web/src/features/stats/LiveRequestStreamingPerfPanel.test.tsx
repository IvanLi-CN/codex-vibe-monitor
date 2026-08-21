/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { LiveRequestStreamingPerf } from "../../lib/api";
import { LiveRequestStreamingPerfPanel } from "./LiveRequestStreamingPerfPanel";

vi.mock("../../i18n", () => ({
  useTranslation: () => ({
    t: (key: string, values?: Record<string, number | string>) => {
      const labels: Record<string, string> = {
        "stats.liveRequestStreaming.title": "Live request streaming",
        "stats.liveRequestStreaming.subtitle": "Buffered control and live-first treatment",
        "stats.liveRequestStreaming.control": "Buffered control",
        "stats.liveRequestStreaming.treatment": "Live-first treatment",
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
        "stats.liveRequestStreaming.routeEofBuffered": "EOF buffered",
        "stats.liveRequestStreaming.routeCacheHit": "Routing cache hit",
        "stats.liveRequestStreaming.routeColdLoad": "Routing cold load",
      };
      if (key === "stats.liveRequestStreaming.insufficient") {
        return `Insufficient samples: ${values?.count} / ${values?.minimum}`;
      }
      if (key === "stats.liveRequestStreaming.coverage") {
        return `Coverage ${values?.rate} · ${values?.count} / ${values?.total}`;
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

function render(data: LiveRequestStreamingPerf) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root?.render(<LiveRequestStreamingPerfPanel data={data} />));
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
    expect(host?.textContent).toContain("4096 / 8192 / 16384 B");
    expect(host?.textContent).toContain("99.5%");
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

    expect(host?.textContent).toContain("Insufficient samples: 8 / 200");
    expect(host?.textContent).not.toContain("+200 ms (+20.0%)");
    expect(host?.textContent).not.toContain("+300 ms (+21.4%)");
  });

  it("keeps the treatment cohort visible when final-route gating buffered it", () => {
    render({
      ...data,
      cohorts: data.cohorts.map((cohort) =>
        cohort.cohort === "treatment" ? { ...cohort, transportMode: "buffered" } : cohort,
      ),
    });

    expect(host?.textContent).toContain("+200 ms (+20.0%)");
    expect(host?.textContent).toContain("+300 ms (+21.4%)");
  });
});
