/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import type {
  TaskMeasurementCapabilities,
  TaskWorkloadTrend as WorkloadTrend,
} from "../../lib/api";
import { resolveWorkloadHint, TaskWorkloadSummary } from "./TaskWorkloadTrend";

const metric = {
  value: 32,
  unit: "invocation rows",
  scope: "retention_archive",
  range: "run",
  observedAt: "2026-10-04T12:00:00.000Z",
  coverage: "exact",
} as const;

const sample = {
  sampleId: "sample-1",
  executionUid: "execution-1",
  managedRunId: 1,
  taskKey: "retention_archive",
  triggerKind: "manual",
  attemptedAt: "2026-10-04T11:59:00.000Z",
  actualStartedAt: "2026-10-04T11:59:00.000Z",
  finishedAt: "2026-10-04T12:00:00.000Z",
  durationMs: 60_000,
  status: "success",
  reason: null,
  sequence: 1,
  pending: metric,
  discovered: metric,
  processed: metric,
  subsetRelation: "equal",
};

const trend: WorkloadTrend = {
  revision: 1,
  coverage: "complete",
  samples: [sample],
  latestPending: metric,
  latestProcessed: metric,
  latestObservedAt: metric.observedAt,
  processingRatePerSecond: null,
  processingRateWindow: null,
  clearanceEta: null,
  clearanceEstimateWindow: null,
  clearanceEstimateCoverage: null,
  clearanceEstimateReason: "insufficient_samples",
};

const unsupportedCapabilities: TaskMeasurementCapabilities = {
  pending: { supported: false, unit: null, scope: null },
  discovered: { supported: false, unit: null, scope: null },
  processed: { supported: false, unit: null, scope: null },
};

const unobservedTrend = {
  ...trend,
  coverage: "some metrics unknown",
  samples: [
    {
      ...sample,
      pending: { ...metric, value: null, coverage: "unknown" },
      discovered: { ...metric, value: null, coverage: "unknown" },
      processed: { ...metric, value: null, coverage: "unknown" },
    },
  ],
};

const hintCases = [
  {
    name: "loading",
    state: "loading" as const,
    error: undefined,
    trend: undefined,
    capabilities: undefined,
    expected: "加载中",
  },
  {
    name: "error",
    state: "error" as const,
    error: "维护库暂不可用",
    trend: undefined,
    capabilities: undefined,
    expected: "读取失败：维护库暂不可用",
  },
  {
    name: "legacy interface",
    state: "ready" as const,
    error: undefined,
    trend: null,
    capabilities: undefined,
    expected: "暂无运行计量",
  },
  {
    name: "counterless task",
    state: "ready" as const,
    error: undefined,
    trend: { ...trend, coverage: "not applicable" },
    capabilities: unsupportedCapabilities,
    expected: "该任务不提供工作量计数",
  },
  {
    name: "empty samples",
    state: "ready" as const,
    error: undefined,
    trend: { ...trend, samples: [] },
    capabilities: undefined,
    expected: "暂无运行样本",
  },
  {
    name: "capture gap",
    state: "ready" as const,
    error: undefined,
    trend: { ...trend, coverage: "incomplete: recorder coverage gap" },
    capabilities: undefined,
    expected: "观测有缺口，计量暂不可用",
  },
  {
    name: "no observed counters",
    state: "ready" as const,
    error: undefined,
    trend: { ...trend, coverage: "incomplete: run has no observed counters" },
    capabilities: undefined,
    expected: "本次运行尚未观测计量",
  },
  {
    name: "some metrics unknown",
    state: "ready" as const,
    error: undefined,
    trend: unobservedTrend,
    capabilities: undefined,
    expected: "部分计量未知",
  },
] as const;

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function renderSummary(summaryTrend: WorkloadTrend) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(<TaskWorkloadSummary trend={summaryTrend} />);
  });
}

describe("TaskWorkloadTrend state contracts", () => {
  afterEach(() => {
    act(() => root?.unmount());
    host?.remove();
    host = null;
    root = null;
  });

  it.each(hintCases)("resolves the $name hint", ({
    state,
    error,
    trend: nextTrend,
    capabilities,
    expected,
  }) => {
    expect(resolveWorkloadHint(state, error, nextTrend, capabilities)).toBe(expected);
  });

  it.each([
    ["stale_observation", "观测已过期，等待新数据"],
    ["task_disabled", "任务已停用，暂停清零预估"],
    ["capture_gap", "观测存在缺口，清零预估暂不可用"],
  ] as const)("keeps exact zero pending state as %s", (reason, message) => {
    renderSummary({
      ...trend,
      latestPending: { ...metric, value: 0 },
      latestProcessed: null,
      clearanceEstimateReason: reason,
    });

    expect(host?.textContent).toContain(message);
    expect(host?.textContent).not.toContain("已清空");
  });
});
