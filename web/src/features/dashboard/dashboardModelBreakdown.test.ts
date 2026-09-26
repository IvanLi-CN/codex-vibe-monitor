/** @vitest-environment jsdom */
import { afterEach, describe, expect, it } from "vitest";
import type { ModelPerformanceModel, UsageBreakdownModel } from "../../lib/api";
import {
  DASHBOARD_MODEL_BREAKDOWN_MODE_STORAGE_KEY,
  DASHBOARD_MODEL_BREAKDOWN_SORT_STORAGE_KEY_PREFIX,
  groupUsageBreakdownModels,
  nextModelSortRule,
  persistDashboardModelBreakdownMode,
  persistDashboardModelBreakdownSort,
  readDashboardModelBreakdownMode,
  readDashboardModelBreakdownSort,
  sortForMode,
  sortModelPerformanceModels,
  sortUsageBreakdownModels,
} from "./dashboardModelBreakdown";

function performanceModel(
  model: string,
  reasoningEffort: string | null,
  cumulativeUsageDurationMs: number | null = 100,
): ModelPerformanceModel {
  return {
    model,
    reasoningEffort,
    tokensPerMinute: 100,
    streamingResponseRate: 100,
    avgResponseMs: 100,
    avgFirstTokenMs: 100,
    wallClockUsageDurationMs: 100,
    cumulativeUsageDurationMs,
    parallelism: 1,
  };
}

function usageModel(
  model: string,
  reasoningEffort: string | null,
  values: Pick<UsageBreakdownModel, "cacheWriteTokens" | "cacheReadTokens" | "outputTokens">,
  costs?: UsageBreakdownModel["costs"],
): UsageBreakdownModel {
  return { model, reasoningEffort, ...values, costs };
}

afterEach(() => {
  window.localStorage.removeItem(DASHBOARD_MODEL_BREAKDOWN_MODE_STORAGE_KEY);
  window.localStorage.removeItem(
    `${DASHBOARD_MODEL_BREAKDOWN_SORT_STORAGE_KEY_PREFIX}.model-performance`,
  );
  window.localStorage.removeItem(
    `${DASHBOARD_MODEL_BREAKDOWN_SORT_STORAGE_KEY_PREFIX}.usage-breakdown`,
  );
});

describe("dashboard model breakdown preferences", () => {
  it("defaults to detailed mode and restores the two independent window sorts", () => {
    expect(readDashboardModelBreakdownMode()).toBe("detailed");
    expect(readDashboardModelBreakdownSort("model-performance")).toEqual({
      column: "cumulative-duration",
      direction: "desc",
    });
    expect(readDashboardModelBreakdownSort("usage-breakdown")).toEqual({
      column: "total",
      direction: "desc",
    });

    persistDashboardModelBreakdownMode("simple");
    persistDashboardModelBreakdownSort("model-performance", {
      column: "model",
      rule: "effort-desc",
    });
    persistDashboardModelBreakdownSort("usage-breakdown", {
      column: "cache-hit-rate",
      direction: "asc",
    });

    expect(readDashboardModelBreakdownMode()).toBe("simple");
    expect(readDashboardModelBreakdownSort("model-performance")).toEqual({
      column: "model",
      rule: "effort-desc",
    });
    expect(readDashboardModelBreakdownSort("usage-breakdown")).toEqual({
      column: "cache-hit-rate",
      direction: "asc",
    });
  });

  it("falls back to a model-name rule when an effort sort is restored in simple mode", () => {
    expect(sortForMode({ column: "model", rule: "effort-desc" }, "simple")).toEqual({
      column: "model",
      rule: "name-asc",
    });
    expect(sortForMode({ column: "model", rule: "effort-desc" }, "detailed")).toEqual({
      column: "model",
      rule: "effort-desc",
    });
  });

  it("cycles model sorting from the table header without a menu", () => {
    expect(nextModelSortRule(null, false)).toBe("name-asc");
    expect(nextModelSortRule("name-asc", false)).toBe("name-desc");
    expect(nextModelSortRule("name-desc", false)).toBe("effort-asc");
    expect(nextModelSortRule("effort-asc", false)).toBe("effort-desc");
    expect(nextModelSortRule("effort-desc", false)).toBe("name-asc");
    expect(nextModelSortRule("name-desc", true)).toBe("name-asc");
  });

  it("treats unknown reasoning efforts as last for either effort direction", () => {
    const models = [
      performanceModel("model-max", "max"),
      performanceModel("model-unknown", "adaptive"),
      performanceModel("model-minimal", "minimal"),
      performanceModel("model-missing", null),
      performanceModel("model-ultra", "ultra"),
    ];

    expect(
      sortModelPerformanceModels(models, { column: "model", rule: "effort-asc" }, "en-US").map(
        (model) => model.model,
      ),
    ).toEqual(["model-minimal", "model-max", "model-ultra", "model-missing", "model-unknown"]);
    expect(
      sortModelPerformanceModels(models, { column: "model", rule: "effort-desc" }, "en-US").map(
        (model) => model.model,
      ),
    ).toEqual(["model-ultra", "model-max", "model-minimal", "model-missing", "model-unknown"]);
  });

  it("keeps missing metric values last while sorting numeric columns", () => {
    const models = [
      performanceModel("missing", "low", null),
      performanceModel("high", "low", 300),
      performanceModel("low", "low", 100),
    ];

    expect(
      sortModelPerformanceModels(
        models,
        { column: "cumulative-duration", direction: "asc" },
        "en-US",
      ).map((model) => model.model),
    ).toEqual(["low", "high", "missing"]);
    expect(
      sortModelPerformanceModels(
        models,
        { column: "cumulative-duration", direction: "desc" },
        "en-US",
      ).map((model) => model.model),
    ).toEqual(["high", "low", "missing"]);
  });
});

describe("simple usage breakdown grouping", () => {
  it("merges every effort for a model and preserves token and cost totals", () => {
    const grouped = groupUsageBreakdownModels([
      usageModel(
        "gpt-5.6",
        "max",
        { cacheWriteTokens: 10, cacheReadTokens: 20, outputTokens: 30 },
        { input: 1, cacheWrite: 2, cacheRead: 3, output: 4, reasoning: 5, unknown: 0 },
      ),
      usageModel(
        "gpt-5.6",
        "low",
        { cacheWriteTokens: 4, cacheReadTokens: 5, outputTokens: 6 },
        { input: 0.5, cacheWrite: 1, cacheRead: 1.5, output: 2, reasoning: 2.5, unknown: 0.25 },
      ),
      usageModel("other", null, { cacheWriteTokens: 1, cacheReadTokens: 0, outputTokens: 0 }),
    ]);

    expect(grouped).toHaveLength(2);
    expect(grouped[0]).toMatchObject({
      model: "gpt-5.6",
      reasoningEffort: null,
      cacheWriteTokens: 14,
      cacheReadTokens: 25,
      outputTokens: 36,
      costs: {
        input: 1.5,
        cacheWrite: 3,
        cacheRead: 4.5,
        output: 6,
        reasoning: 7.5,
        unknown: 0.25,
      },
    });
  });

  it("sorts usage rows by tokens for metric rules and by name for model rules", () => {
    const models = [
      usageModel("z-model", "high", { cacheWriteTokens: 0, cacheReadTokens: 0, outputTokens: 2 }),
      usageModel("a-model", "low", { cacheWriteTokens: 0, cacheReadTokens: 0, outputTokens: 10 }),
    ];

    expect(
      sortUsageBreakdownModels(models, { column: "total", direction: "desc" }, "en-US").map(
        (model) => model.model,
      ),
    ).toEqual(["a-model", "z-model"]);
    expect(
      sortUsageBreakdownModels(models, { column: "model", rule: "name-asc" }, "en-US").map(
        (model) => model.model,
      ),
    ).toEqual(["a-model", "z-model"]);
  });
});
